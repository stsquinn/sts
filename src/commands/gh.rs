use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;

use crate::config::Config;
use crate::{platform, run};

#[derive(Subcommand)]
pub enum Command {
    /// Clone every repository of a GitHub organization, or pull the ones already cloned.
    CloneOrg(CloneOrgArgs),
}

#[derive(Args)]
pub struct CloneOrgArgs {
    /// GitHub organization or user. Falls back to `github.org` in the config.
    org: Option<String>,
    /// Destination directory [default: ./<ORG>].
    #[arg(short, long)]
    dir: Option<PathBuf>,
    /// Parallel clones.
    #[arg(short, long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..))]
    jobs: u32,
    /// Clone protocol.
    #[arg(short, long, value_enum, default_value_t = Protocol::Ssh)]
    protocol: Protocol,
    /// Only repositories with this visibility.
    #[arg(short, long, value_enum)]
    visibility: Option<Visibility>,
    /// Maximum repositories to fetch.
    #[arg(short, long, default_value_t = 1000)]
    limit: u32,
    /// Include archived repositories.
    #[arg(long)]
    include_archived: bool,
    /// Include forks.
    #[arg(long)]
    include_forks: bool,
    /// Create shallow clones with this depth.
    #[arg(long)]
    depth: Option<u32>,
    /// Print what would happen without touching the disk.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Protocol {
    Ssh,
    Https,
}

#[derive(Clone, Copy, ValueEnum)]
enum Visibility {
    Public,
    Private,
    Internal,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repo {
    name: String,
    ssh_url: String,
    url: String,
}

struct Job {
    name: String,
    url: String,
    target: PathBuf,
    update: bool,
}

pub fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::CloneOrg(args) => clone_org(args, cfg),
    }
}

fn clone_org(args: CloneOrgArgs, cfg: &Config) -> Result<()> {
    let org = args
        .org
        .or_else(|| cfg.github.org.clone())
        .context("organization is required: pass <ORG> or set github.org in the config")?;
    platform::require("git")?;
    platform::require("gh")?;
    run::capture(Process::new("gh").args(["auth", "status"]))
        .context("not logged in to GitHub; run `gh auth login` first")?;

    let mut list = Process::new("gh");
    list.args([
        "repo",
        "list",
        &org,
        "--limit",
        &args.limit.to_string(),
        "--json",
        "name,sshUrl,url",
    ]);
    if let Some(v) = args.visibility {
        list.args(["--visibility", v.to_possible_value().unwrap().get_name()]);
    }
    if !args.include_archived {
        list.arg("--no-archived");
    }
    if !args.include_forks {
        list.arg("--source");
    }
    eprintln!("Fetching repository list for '{org}'...");
    let repos: Vec<Repo> = serde_json::from_str(&run::capture(&mut list)?)
        .context("unexpected `gh repo list` output")?;

    let dest = args.dir.unwrap_or_else(|| PathBuf::from(&org));
    if repos.is_empty() {
        println!("No repositories found for '{org}' with the given filters.");
        return Ok(());
    }
    let jobs = plan(repos, &dest, args.protocol);
    println!(
        "Found {}. Destination: {}",
        repo_count(jobs.len()),
        dest.display()
    );

    if args.dry_run {
        for job in &jobs {
            if job.update {
                println!("  [update] {}", job.name);
            } else {
                println!("  [clone ] {} <- {}", job.name, job.url);
            }
        }
        return Ok(());
    }

    fs::create_dir_all(&dest).with_context(|| format!("cannot create {}", dest.display()))?;
    let next = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let workers = (args.jobs as usize).min(jobs.len());
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                while let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if !sync(job, args.depth) {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });

    match failed.into_inner() {
        0 => {
            println!(
                "Done. All {} are in {}",
                repo_count(jobs.len()),
                dest.display()
            );
            Ok(())
        }
        n => bail!("{} failed to clone. See the errors above.", repo_count(n)),
    }
}

fn repo_count(n: usize) -> String {
    if n == 1 {
        "1 repository".to_string()
    } else {
        format!("{n} repositories")
    }
}

fn plan(repos: Vec<Repo>, dest: &Path, protocol: Protocol) -> Vec<Job> {
    repos
        .into_iter()
        .map(|repo| {
            let target = dest.join(&repo.name);
            Job {
                update: target.join(".git").is_dir(),
                url: if protocol == Protocol::Ssh {
                    repo.ssh_url
                } else {
                    repo.url
                },
                name: repo.name,
                target,
            }
        })
        .collect()
}

/// Returns false only when a fresh clone fails. A pull that cannot fast-forward is skipped.
fn sync(job: &Job, depth: Option<u32>) -> bool {
    let mut git = Process::new("git");
    git.env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());

    if job.update {
        git.arg("-C")
            .arg(&job.target)
            .args(["pull", "--ff-only", "--quiet"])
            .stderr(Stdio::null());
        if git.status().is_ok_and(|s| s.success()) {
            println!("updated  {}", job.name);
        } else {
            eprintln!("skipped  {} (local changes or diverged branch)", job.name);
        }
        return true;
    }

    git.args(["clone", "--quiet"]);
    if let Some(depth) = depth {
        git.arg("--depth").arg(depth.to_string());
    }
    git.arg(&job.url).arg(&job.target);
    if git.status().is_ok_and(|s| s.success()) {
        println!("cloned   {}", job.name);
        true
    } else {
        eprintln!("FAILED   {}", job.name);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GH_OUTPUT: &str = r#"[
        {"name":"api","sshUrl":"git@github.com:acme/api.git","url":"https://github.com/acme/api"},
        {"name":"web","sshUrl":"git@github.com:acme/web.git","url":"https://github.com/acme/web"}
    ]"#;

    #[test]
    fn plans_ssh_and_https_urls() {
        let dest = Path::new("/nonexistent/acme");
        let ssh = plan(
            serde_json::from_str(GH_OUTPUT).unwrap(),
            dest,
            Protocol::Ssh,
        );
        assert_eq!(ssh[0].url, "git@github.com:acme/api.git");
        assert_eq!(ssh[1].target, dest.join("web"));
        assert!(!ssh[0].update);

        let https = plan(
            serde_json::from_str(GH_OUTPUT).unwrap(),
            dest,
            Protocol::Https,
        );
        assert_eq!(https[1].url, "https://github.com/acme/web");
    }

    #[test]
    fn counts_repositories_in_singular_and_plural() {
        assert_eq!(repo_count(1), "1 repository");
        assert_eq!(repo_count(3), "3 repositories");
    }
}

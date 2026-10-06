use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use dialoguer::{Input, Password};

use crate::config::{self, Config};
use crate::platform::{self, Engine};
use crate::run;

const DEFAULT_IMAGE: &str = "sonarsource/sonarqube-scan:3.0.2";

#[derive(Subcommand)]
pub enum Command {
    /// Scan a local project with the SonarQube scanner in a container.
    ///
    /// Asks for every value that is not passed as a flag, prefilled from the environment and the config.
    /// The token comes from SONAR_TOKEN or a hidden prompt. It is never a flag, so it stays out of shell history.
    Scan(ScanArgs),
}

#[derive(Args)]
pub struct ScanArgs {
    /// Project directory to scan [default: the current directory].
    path: Option<PathBuf>,
    /// SonarQube project key [default: the directory name].
    #[arg(short = 'k', long)]
    project_key: Option<String>,
    /// SonarQube server URL [default: SONAR_HOST_URL, then `sonar.host_url` in the config].
    #[arg(long)]
    host_url: Option<String>,
    /// Scanner image. Falls back to `sonar.image` in the config.
    #[arg(long, env = "SONAR_SCANNER_IMAGE")]
    image: Option<String>,
    /// Container engine. Falls back to `sonar.engine` in the config, then to auto-detection.
    #[arg(long, value_enum)]
    engine: Option<Engine>,
}

pub fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::Scan(args) => scan(args, cfg),
    }
}

fn scan(args: ScanArgs, cfg: &Config) -> Result<()> {
    // Prompts draw on stderr, so both ends must be a terminal.
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();

    let host_default = env_var("SONAR_HOST_URL").or_else(|| cfg.sonar.host_url.clone());
    let tip = interactive && args.host_url.is_none() && host_default.is_none();
    let host_url = match args.host_url {
        Some(url) => url,
        None if interactive => ask("SonarQube URL", host_default, check_host_url)?,
        None => host_default.context(
            "SonarQube URL is not set: pass --host-url, export SONAR_HOST_URL or set sonar.host_url in the config",
        )?,
    };
    check_host_url(&host_url).map_err(anyhow::Error::msg)?;
    let host_url = host_url.trim_end_matches('/').to_string();
    if tip {
        eprintln!(
            "Tip: set sonar.host_url in {} to prefill this prompt.",
            config::path().display()
        );
    }
    if host_url.starts_with("http://") {
        eprintln!("warning: {host_url} uses plain HTTP, so the token is sent unencrypted.");
    }

    let path = match args.path {
        Some(path) => path,
        None if interactive => {
            let cwd = std::env::current_dir()?.display().to_string();
            let answer = ask("Project path", Some(cwd), |p| {
                project_dir(&expand_tilde(p))
                    .map(drop)
                    .map_err(|e| format!("{e:#}"))
            })?;
            expand_tilde(&answer)
        }
        None => PathBuf::from("."),
    };
    let project = project_dir(&path)?;

    let key_default = project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    let project_key = match args.project_key {
        Some(key) => key,
        None if interactive => ask("SonarQube project key", key_default, check_project_key)?,
        // A guessed key would silently create a new project on the server.
        None => bail!("pass --project-key when there is no terminal to prompt for it"),
    };
    check_project_key(&project_key).map_err(anyhow::Error::msg)?;

    let token = match env_var("SONAR_TOKEN") {
        Some(token) => token,
        None if interactive => Password::new().with_prompt("SonarQube token").interact()?,
        None => bail!("SONAR_TOKEN is not set and there is no terminal to prompt for it"),
    };
    if token.is_empty() {
        bail!("SonarQube token is required");
    }

    let image = args
        .image
        .or_else(|| cfg.sonar.image.clone())
        .unwrap_or_else(|| DEFAULT_IMAGE.to_string());
    let engine = match args.engine.or(cfg.sonar.engine) {
        Some(engine) => engine,
        None => Engine::detect()?,
    };
    platform::require(engine.bin())?;

    eprintln!(
        "Scanning '{}' as SonarQube project '{project_key}' on {host_url} with {}...",
        project.display(),
        engine.bin()
    );
    // Secrets go through the environment so they never appear in the process list.
    run::status(
        Process::new(engine.bin())
            .args(container_args(
                engine,
                &project,
                &image,
                platform::selinux_enforcing(),
            ))
            .env("SONAR_HOST_URL", host_url)
            .env("SONAR_TOKEN", token)
            .env(
                "SONAR_SCANNER_OPTS",
                format!("-Dsonar.projectKey={project_key}"),
            ),
    )
}

/// Prompts with `default` prefilled and asks again until `check` passes.
fn ask(
    prompt: &str,
    default: Option<String>,
    check: impl Fn(&str) -> Result<(), String>,
) -> Result<String> {
    let mut input = Input::<String>::new()
        .with_prompt(prompt)
        .validate_with(|v: &String| check(v.trim()));
    if let Some(default) = default {
        input = input.default(default);
    }
    Ok(input.interact_text()?.trim().to_string())
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn check_host_url(url: &str) -> Result<(), String> {
    match url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        Some(rest) if !rest.is_empty() && !rest.contains(char::is_whitespace) => Ok(()),
        _ => Err(format!(
            "'{url}' is not a URL such as https://sonar.example.com"
        )),
    }
}

/// SonarQube allows letters, digits, `-`, `_`, `.` and `:`, with at least one non-digit.
fn check_project_key(key: &str) -> Result<(), String> {
    let allowed = key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c));
    if allowed && key.chars().any(|c| !c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(format!(
            "'{key}' is not a valid key: use letters, digits, '-', '_', '.' or ':', with at least one non-digit"
        ))
    }
}

/// The shell does not expand `~` in a prompt answer, so do it here.
fn expand_tilde(path: &str) -> PathBuf {
    match (path.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(path),
    }
}

fn project_dir(path: &Path) -> Result<PathBuf> {
    let project = path
        .canonicalize()
        .with_context(|| format!("project path is not accessible: {}", path.display()))?;
    check_project_dir(&project)?;
    Ok(project)
}

fn check_project_dir(project: &Path) -> Result<()> {
    if !project.is_dir() {
        bail!("project path is not a directory: {}", project.display());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    // On SELinux hosts the mount is relabeled recursively, which must never hit / or $HOME.
    if project == Path::new("/") || Some(project) == home.as_deref() {
        bail!(
            "refusing to scan {}; pass a project directory",
            project.display()
        );
    }
    if project.to_string_lossy().contains(':') {
        bail!("project path must not contain ':' because it breaks the volume syntax");
    }
    Ok(())
}

fn container_args(engine: Engine, project: &Path, image: &str, selinux: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "run",
        "--rm",
        "-e",
        "SONAR_HOST_URL",
        "-e",
        "SONAR_TOKEN",
        "-e",
        "SONAR_SCANNER_OPTS",
    ]
    .map(String::from)
    .into();
    // Rootless podman maps container users to sub-UIDs; keep-id lets the scanner write .scannerwork.
    if engine == Engine::Podman {
        args.push("--userns=keep-id".to_string());
    }
    let label = if selinux { ":z" } else { "" };
    args.push("-v".to_string());
    args.push(format!("{}:/usr/src{label}", project.display()));
    args.push(image.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_on_ubuntu_mounts_plainly() {
        let args = container_args(Engine::Docker, Path::new("/work/app"), "img:1", false);
        assert!(!args.iter().any(|a| a.starts_with("--userns")));
        assert!(args.contains(&"/work/app:/usr/src".to_string()));
        assert_eq!(args.last().unwrap(), "img:1");
    }

    #[test]
    fn podman_on_fedora_keeps_uid_and_relabels() {
        let args = container_args(Engine::Podman, Path::new("/work/app"), "img:1", true);
        assert!(args.contains(&"--userns=keep-id".to_string()));
        assert!(args.contains(&"/work/app:/usr/src:z".to_string()));
    }

    #[test]
    fn rejects_root_and_home() {
        assert!(check_project_dir(Path::new("/")).is_err());
        if let Some(home) = std::env::var_os("HOME") {
            assert!(check_project_dir(Path::new(&home)).is_err());
        }
    }

    #[test]
    fn accepts_only_http_urls() {
        assert!(check_host_url("https://sonar.example.com").is_ok());
        assert!(check_host_url("http://sonar.local:9000").is_ok());
        assert!(check_host_url("sonar.example.com").is_err());
        assert!(check_host_url("https://").is_err());
        assert!(check_host_url("ftp://sonar.example.com").is_err());
    }

    #[test]
    fn validates_project_keys() {
        assert!(check_project_key("my-app").is_ok());
        assert!(check_project_key("org:my_app.v2").is_ok());
        assert!(check_project_key("").is_err());
        assert!(check_project_key("123").is_err());
        assert!(check_project_key("my app").is_err());
    }

    #[test]
    fn expands_tilde_to_home() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(expand_tilde("~/src/app"), home.join("src/app"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("/abs/~x"), PathBuf::from("/abs/~x"));
    }
}

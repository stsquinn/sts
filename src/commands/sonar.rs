use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use dialoguer::{Input, Password};

use crate::config::Config;
use crate::platform::{self, Engine};
use crate::run;

const DEFAULT_IMAGE: &str = "sonarsource/sonarqube-scan:3.0.2";

#[derive(Subcommand)]
pub enum Command {
    /// Scan a local project with the SonarQube scanner in a container.
    ///
    /// The token comes from SONAR_TOKEN or a hidden prompt. It is never a flag, so it stays out of shell history.
    Scan(ScanArgs),
}

#[derive(Args)]
pub struct ScanArgs {
    /// Project directory to scan.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// SonarQube project key. Prompted when omitted.
    #[arg(short = 'k', long)]
    project_key: Option<String>,
    /// SonarQube server URL. Falls back to `sonar.host_url` in the config.
    #[arg(long, env = "SONAR_HOST_URL")]
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
    let host_url = args.host_url.or_else(|| cfg.sonar.host_url.clone()).context(
        "SonarQube URL is not set: pass --host-url, export SONAR_HOST_URL or set sonar.host_url in the config",
    )?;
    let image = args
        .image
        .or_else(|| cfg.sonar.image.clone())
        .unwrap_or_else(|| DEFAULT_IMAGE.to_string());
    let engine = match args.engine.or(cfg.sonar.engine) {
        Some(engine) => engine,
        None => Engine::detect()?,
    };
    platform::require(engine.bin())?;

    let project = args
        .path
        .canonicalize()
        .with_context(|| format!("project path is not accessible: {}", args.path.display()))?;
    check_project_dir(&project)?;

    let project_key = match args.project_key {
        Some(key) => key,
        None => {
            let dir_name = project
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            Input::new()
                .with_prompt("SonarQube project key")
                .default(dir_name)
                .interact_text()?
        }
    };
    if project_key.trim().is_empty() {
        bail!("SonarQube project key is required");
    }
    let token = match std::env::var("SONAR_TOKEN") {
        Ok(token) if !token.is_empty() => token,
        _ => Password::new().with_prompt("SonarQube token").interact()?,
    };
    if token.is_empty() {
        bail!("SonarQube token is required");
    }

    eprintln!(
        "Scanning '{}' as SonarQube project '{project_key}' with {}...",
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
}

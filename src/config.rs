use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Deserialize;

use crate::platform::Engine;

const EXAMPLE: &str = include_str!("../config.example.toml");

/// Values from `~/.config/sts/config.toml`. Flags and environment variables override them.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub aws: AwsConfig,
    pub github: GithubConfig,
    pub sonar: SonarConfig,
    pub gnome: GnomeConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AwsConfig {
    pub region: Option<String>,
    pub profile: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GithubConfig {
    pub org: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SonarConfig {
    pub host_url: Option<String>,
    pub image: Option<String>,
    pub engine: Option<Engine>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GnomeConfig {
    pub workspaces: Option<Vec<String>>,
}

/// `$STS_CONFIG`, else `$XDG_CONFIG_HOME/sts/config.toml`, else `~/.config/sts/config.toml`.
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("STS_CONFIG").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("sts").join("config.toml")
}

pub fn load() -> Result<Config> {
    let path = path();
    match fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

#[derive(Subcommand)]
pub enum Command {
    /// Print the config file path.
    Path,
    /// Create the config file from the example. Never overwrites an existing file.
    Init,
}

pub fn run(cmd: Command) -> Result<()> {
    let path = path();
    match cmd {
        Command::Path => println!("{}", path.display()),
        Command::Init => {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)
                    .with_context(|| format!("cannot create {}", dir.display()))?;
            }
            let mut file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(f) => f,
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    println!("{} already exists. Edit it directly.", path.display());
                    return Ok(());
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("cannot create {}", path.display()));
                }
            };
            file.write_all(EXAMPLE.as_bytes())?;
            println!("Created {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        toml::from_str::<Config>(EXAMPLE).unwrap();
    }

    #[test]
    fn full_config_parses() {
        let cfg: Config = toml::from_str(
            r#"
            [aws]
            region = "ap-southeast-2"
            [sonar]
            host_url = "https://sonar.example.com"
            engine = "podman"
            [gnome]
            workspaces = ["A", "B"]
            "#,
        )
        .unwrap();
        assert_eq!(cfg.aws.region.as_deref(), Some("ap-southeast-2"));
        assert_eq!(cfg.sonar.engine, Some(Engine::Podman));
        assert_eq!(cfg.gnome.workspaces.unwrap().len(), 2);
    }

    #[test]
    fn typos_are_rejected() {
        assert!(toml::from_str::<Config>("[aws]\nregoin = \"x\"\n").is_err());
    }
}

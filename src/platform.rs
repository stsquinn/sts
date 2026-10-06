use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Ubuntu, Debian and derivatives (apt).
    Debian,
    /// Fedora, RHEL and derivatives (dnf).
    Fedora,
    Other,
}

#[derive(Debug, Clone)]
pub struct Distro {
    pub name: String,
    pub family: Family,
}

impl Distro {
    pub fn detect() -> Self {
        let text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        Self::parse(&text)
    }

    pub fn parse(os_release: &str) -> Self {
        let field = |key: &str| {
            os_release.lines().find_map(|line| {
                let value = line.strip_prefix(key)?.strip_prefix('=')?;
                Some(value.trim().trim_matches('"').to_string())
            })
        };
        let id = field("ID").unwrap_or_default();
        let like = field("ID_LIKE").unwrap_or_default();
        let ids: Vec<&str> = std::iter::once(id.as_str())
            .chain(like.split_whitespace())
            .collect();

        let family = if ids.iter().any(|i| matches!(*i, "ubuntu" | "debian")) {
            Family::Debian
        } else if ids
            .iter()
            .any(|i| matches!(*i, "fedora" | "rhel" | "centos"))
        {
            Family::Fedora
        } else {
            Family::Other
        };
        let name = field("PRETTY_NAME")
            .or_else(|| (!id.is_empty()).then_some(id))
            .unwrap_or_else(|| "unknown Linux".to_string());
        Self { name, family }
    }
}

const SSM_PLUGIN_URL: &str = "https://s3.amazonaws.com/session-manager-downloads/plugin/latest";

/// The install command for `bin` on this distro family, if sts knows one.
pub fn install_hint(bin: &str, family: Family) -> Option<String> {
    let arm = std::env::consts::ARCH == "aarch64";
    let hint = match (bin, family) {
        ("git", Family::Debian) => "sudo apt install git".to_string(),
        ("git", Family::Fedora) => "sudo dnf install git".to_string(),
        ("gh", Family::Debian) => "sudo apt install gh".to_string(),
        ("gh", Family::Fedora) => "sudo dnf install gh".to_string(),
        // The snap's latest/stable channel is AWS CLI v1.
        ("aws", Family::Debian) => {
            "sudo snap install aws-cli --classic --channel=v2/stable".to_string()
        }
        ("aws", Family::Fedora) => "sudo dnf install awscli2".to_string(),
        ("session-manager-plugin", Family::Debian) => format!(
            "curl -fsSLo /tmp/smp.deb {SSM_PLUGIN_URL}/{}/session-manager-plugin.deb && sudo apt install /tmp/smp.deb",
            if arm { "ubuntu_arm64" } else { "ubuntu_64bit" }
        ),
        ("session-manager-plugin", Family::Fedora) => format!(
            "sudo dnf install {SSM_PLUGIN_URL}/{}/session-manager-plugin.rpm",
            if arm { "linux_arm64" } else { "linux_64bit" }
        ),
        ("docker", Family::Debian) => "sudo apt install docker.io".to_string(),
        ("docker", Family::Fedora) => "sudo dnf install moby-engine".to_string(),
        ("podman", Family::Debian) => "sudo apt install podman".to_string(),
        ("podman", Family::Fedora) => "sudo dnf install podman".to_string(),
        ("gsettings", Family::Debian) => "sudo apt install libglib2.0-bin".to_string(),
        ("gsettings", Family::Fedora) => "sudo dnf install glib2".to_string(),
        _ => return None,
    };
    Some(hint)
}

/// Finds `bin` on PATH, or fails with the install command for this distro.
pub fn require(bin: &str) -> Result<PathBuf> {
    if let Ok(path) = which::which(bin) {
        return Ok(path);
    }
    let distro = Distro::detect();
    match install_hint(bin, distro.family) {
        Some(hint) => bail!(
            "`{bin}` is not installed. On {}, install it with:\n  {hint}",
            distro.name
        ),
        None => bail!("`{bin}` is not installed or not in PATH"),
    }
}

pub fn selinux_enforcing() -> bool {
    std::fs::read_to_string("/sys/fs/selinux/enforce").is_ok_and(|s| s.trim() == "1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Docker,
    Podman,
}

impl Engine {
    pub fn bin(self) -> &'static str {
        match self {
            Engine::Docker => "docker",
            Engine::Podman => "podman",
        }
    }

    /// Prefers Docker, unless `docker` is the podman-docker shim that Fedora ships.
    pub fn detect() -> Result<Self> {
        if which::which("docker").is_ok() && !docker_is_podman_shim() {
            return Ok(Engine::Docker);
        }
        if which::which("podman").is_ok() {
            return Ok(Engine::Podman);
        }
        let distro = Distro::detect();
        let preferred = match distro.family {
            Family::Fedora => "podman",
            _ => "docker",
        };
        match install_hint(preferred, distro.family) {
            Some(hint) => bail!(
                "no container engine found. On {}, install one with:\n  {hint}",
                distro.name
            ),
            None => bail!("no container engine found; install docker or podman"),
        }
    }
}

fn docker_is_podman_shim() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .is_ok_and(|out| {
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains("podman")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ubuntu() {
        let d = Distro::parse(
            "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu 24.04.5 LTS\"\nVERSION_ID=\"24.04\"\n",
        );
        assert_eq!(d.family, Family::Debian);
        assert_eq!(d.name, "Ubuntu 24.04.5 LTS");
    }

    #[test]
    fn detects_fedora() {
        let d = Distro::parse(
            "NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=42\nPRETTY_NAME=\"Fedora Linux 42 (Workstation Edition)\"\n",
        );
        assert_eq!(d.family, Family::Fedora);
    }

    #[test]
    fn detects_derivatives_through_id_like() {
        assert_eq!(
            Distro::parse("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n").family,
            Family::Debian
        );
        assert_eq!(
            Distro::parse("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n").family,
            Family::Fedora
        );
    }

    #[test]
    fn unknown_distro_falls_back() {
        let d = Distro::parse("ID=arch\n");
        assert_eq!(d.family, Family::Other);
        assert_eq!(d.name, "arch");
        assert_eq!(Distro::parse("").name, "unknown Linux");
    }

    #[test]
    fn every_tool_has_hints_for_both_families() {
        for bin in [
            "git",
            "gh",
            "aws",
            "session-manager-plugin",
            "docker",
            "podman",
            "gsettings",
        ] {
            let debian = install_hint(bin, Family::Debian).unwrap();
            assert!(debian.contains("apt") || debian.contains("snap"), "{bin}");
            assert!(
                install_hint(bin, Family::Fedora).unwrap().contains("dnf"),
                "{bin}"
            );
            assert!(install_hint(bin, Family::Other).is_none(), "{bin}");
        }
    }
}

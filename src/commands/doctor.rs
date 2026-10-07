use anyhow::{Result, bail};

use crate::platform::{self, Distro, Family};

const TOOLS: [(&str, &str); 7] = [
    ("git", "gh clone-org"),
    ("gh", "gh clone-org"),
    ("aws", "ssm connect, ssm enable"),
    ("session-manager-plugin", "ssm connect"),
    ("gsettings", "gnome workspaces"),
    ("bluetoothctl", "bluetooth pair, bluetooth connect"),
    ("btmgmt", "bluetooth pair"),
];

pub fn run() -> Result<()> {
    let distro = Distro::detect();
    let family = match distro.family {
        Family::Debian => "apt",
        Family::Fedora => "dnf",
        Family::Other => "unsupported, no install hints",
    };
    println!("Distro:  {} ({family})", distro.name);
    println!(
        "SELinux: {}",
        if platform::selinux_enforcing() {
            "enforcing, so sonar scan relabels its mount with :z"
        } else {
            "not enforcing"
        }
    );
    println!();
    println!("{:<24} {:<8} USED BY", "TOOL", "STATUS");

    let mut missing = 0;
    let mut hints = Vec::new();
    for (bin, used_by) in TOOLS {
        let found = which::which(bin).is_ok();
        println!(
            "{bin:<24} {:<8} {used_by}",
            if found { "ok" } else { "missing" }
        );
        if !found {
            missing += 1;
            hints.extend(platform::install_hint(bin, distro.family));
        }
    }

    let engine = platform::Engine::detect().ok();
    println!(
        "{:<24} {:<8} sonar scan",
        engine.map_or("docker or podman", |e| e.bin()),
        if engine.is_some() { "ok" } else { "missing" }
    );
    if engine.is_none() {
        missing += 1;
        let preferred = if distro.family == Family::Fedora {
            "podman"
        } else {
            "docker"
        };
        hints.extend(platform::install_hint(preferred, distro.family));
    }

    if missing == 0 {
        println!("\nAll tools are installed.");
        return Ok(());
    }
    if !hints.is_empty() {
        println!("\nInstall the missing tools:");
        for hint in &hints {
            println!("  {hint}");
        }
    }
    bail!("{missing} tool(s) missing")
}

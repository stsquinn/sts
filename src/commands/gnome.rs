use std::process::Command as Process;

use anyhow::Result;
use clap::Subcommand;

use crate::config::Config;
use crate::{platform, run};

const DEFAULT_WORKSPACES: [&str; 4] = ["Personal", "Project01", "Project02", "Project03"];

#[derive(Subcommand)]
pub enum Command {
    /// Set the GNOME workspace names.
    Workspaces {
        /// Workspace names. Falls back to `gnome.workspaces` in the config, then to built-in defaults.
        names: Vec<String>,
    },
}

pub fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::Workspaces { names } => {
            let names = if !names.is_empty() {
                names
            } else if let Some(names) = &cfg.gnome.workspaces {
                names.clone()
            } else {
                DEFAULT_WORKSPACES.map(String::from).to_vec()
            };
            platform::require("gsettings")?;
            run::status(Process::new("gsettings").args([
                "set",
                "org.gnome.desktop.wm.preferences",
                "workspace-names",
                &gvariant_strv(&names),
            ]))?;
            println!("Workspace names set: {}", names.join(", "));
            Ok(())
        }
    }
}

/// Formats names as a GVariant string array, for example `['a', 'b']`.
fn gvariant_strv(names: &[String]) -> String {
    let quoted: Vec<String> = names
        .iter()
        .map(|n| format!("'{}'", n.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_escapes_names() {
        let names = vec!["Personal".to_string(), "Quinn's".to_string()];
        assert_eq!(gvariant_strv(&names), r"['Personal', 'Quinn\'s']");
    }
}

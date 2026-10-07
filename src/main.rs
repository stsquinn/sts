mod commands;
mod config;
mod platform;
mod run;

use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

/// Utility CLI that automates day-to-day company work on Ubuntu and Fedora.
#[derive(Parser)]
#[command(name = "sts", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// GitHub helpers.
    #[command(subcommand)]
    Gh(commands::gh::Command),
    /// AWS Systems Manager helpers for EC2.
    #[command(subcommand)]
    Ssm(commands::ssm::Command),
    /// SonarQube helpers.
    #[command(subcommand)]
    Sonar(commands::sonar::Command),
    /// GNOME desktop helpers.
    #[command(subcommand)]
    Gnome(commands::gnome::Command),
    /// Bluetooth helpers.
    #[command(subcommand, visible_alias = "bt")]
    Bluetooth(commands::bluetooth::Command),
    /// Check required tools and print install commands for this distro.
    Doctor,
    /// Manage the sts config file.
    #[command(subcommand)]
    Config(config::Command),
    /// Print a shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

fn main() -> ExitCode {
    // dialoguer hides the cursor while prompting; restore it if the user presses Ctrl-C.
    let _ = ctrlc::set_handler(|| {
        eprint!("\x1b[?25h");
        std::process::exit(130);
    });

    match dispatch(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Gh(cmd) => commands::gh::run(cmd, &config::load()?),
        Command::Ssm(cmd) => commands::ssm::run(cmd, &config::load()?),
        Command::Sonar(cmd) => commands::sonar::run(cmd, &config::load()?),
        Command::Gnome(cmd) => commands::gnome::run(cmd, &config::load()?),
        Command::Bluetooth(cmd) => commands::bluetooth::run(cmd),
        Command::Doctor => commands::doctor::run(),
        Command::Config(cmd) => config::run(cmd),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "sts", &mut std::io::stdout());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_is_well_formed() {
        Cli::command().debug_assert();
    }
}

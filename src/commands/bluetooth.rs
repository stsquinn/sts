use std::process::Command as Process;

use anyhow::{Result, bail};
use clap::Subcommand;
use dialoguer::FuzzySelect;

use crate::{platform, run};

#[derive(Subcommand)]
pub enum Command {
    /// Pair a device by address, then trust and connect it.
    ///
    /// It skips discovery, so it works when a scan cannot find the device.
    /// Put the device in pairing mode first. Pairing runs `sudo btmgmt`.
    Pair {
        /// Device address, for example AA:BB:CC:DD:EE:FF.
        address: String,
    },
    /// Connect a paired device.
    Connect {
        /// Device address. Omit it to pick one of the paired devices.
        address: Option<String>,
    },
}

pub fn run(cmd: Command) -> Result<()> {
    platform::require("bluetoothctl")?;
    match cmd {
        Command::Pair { address } => pair(&parse_address(&address)?),
        Command::Connect { address } => {
            let address = match address {
                Some(address) => parse_address(&address)?,
                None => match pick()? {
                    Some(address) => address,
                    None => {
                        println!("Cancelled.");
                        return Ok(());
                    }
                },
            };
            connect(&address)
        }
    }
}

fn pair(address: &str) -> Result<()> {
    if is_paired(address) {
        println!("{address} is already paired.");
    } else {
        platform::require("btmgmt")?;
        eprintln!("Pairing with {address}. The device must be in pairing mode...");
        // -c 3 is NoInputNoOutput and -t 0 is BR/EDR, which is what headsets expect.
        run::status(Process::new("sudo").args(["btmgmt", "pair", "-c", "3", "-t", "0", address]))?;
    }
    run::status(Process::new("bluetoothctl").args(["trust", address]))?;
    connect(address)
}

fn connect(address: &str) -> Result<()> {
    if !is_paired(address) {
        bail!("{address} is not paired. Pair it first with:\n  sts bluetooth pair {address}");
    }
    run::status(Process::new("bluetoothctl").args(["connect", address]))
}

/// `bluetoothctl info` fails for unknown devices, which counts as unpaired.
fn is_paired(address: &str) -> bool {
    run::capture(Process::new("bluetoothctl").args(["info", address]))
        .is_ok_and(|out| out.lines().any(|line| line.trim() == "Paired: yes"))
}

fn pick() -> Result<Option<String>> {
    let out = run::capture(Process::new("bluetoothctl").args(["devices", "Paired"]))?;
    let devices = parse_devices(&out);
    if devices.is_empty() {
        bail!("no paired devices. Pair one first with:\n  sts bluetooth pair <ADDRESS>");
    }
    let index = FuzzySelect::new()
        .with_prompt("Device to connect (type to filter, Esc to cancel)")
        .items(
            devices
                .iter()
                .map(|(address, name)| format!("{name} ({address})")),
        )
        .default(0)
        .interact_opt()?;
    Ok(index.map(|i| devices[i].0.clone()))
}

/// Parses the `Device <address> <name>` lines that `bluetoothctl devices` prints.
fn parse_devices(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            Some((parse_address(address).ok()?, name.trim().to_string()))
        })
        .collect()
}

/// Accepts `AA:BB:CC:DD:EE:FF` in any case and returns it in uppercase.
fn parse_address(input: &str) -> Result<String> {
    let parts: Vec<&str> = input.split(':').collect();
    let valid = parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    if !valid {
        bail!("`{input}` is not a Bluetooth address such as AA:BB:CC:DD:EE:FF");
    }
    Ok(input.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_normalizes_addresses() {
        assert_eq!(
            parse_address("aa:bb:cc:dd:ee:0f").unwrap(),
            "AA:BB:CC:DD:EE:0F"
        );
    }

    #[test]
    fn rejects_malformed_addresses() {
        for bad in [
            "",
            "AA:BB:CC:DD:EE",
            "AA:BB:CC:DD:EE:FF:00",
            "AA-BB-CC-DD-EE-FF",
            "AA:BB:CC:DD:EE:GG",
            "AAA:BB:CC:DD:EE:F",
        ] {
            assert!(parse_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_paired_devices() {
        let devices = parse_devices(
            "Device AA:BB:CC:DD:EE:FF EDIFIER Comfo Run\nDevice 00:11:22:33:44:55\nnoise line\n",
        );
        assert_eq!(
            devices,
            vec![
                (
                    "AA:BB:CC:DD:EE:FF".to_string(),
                    "EDIFIER Comfo Run".to_string()
                ),
                ("00:11:22:33:44:55".to_string(), String::new()),
            ]
        );
    }
}

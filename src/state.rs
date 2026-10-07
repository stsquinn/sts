use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Values sts remembers between runs. Unlike the config, sts writes this file itself.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub bluetooth: BluetoothState,
    pub sonar: SonarState,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BluetoothState {
    pub device: Option<String>,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SonarState {
    pub host_url: Option<String>,
}

/// `$XDG_STATE_HOME/sts/state.toml`, else `~/.local/state/sts/state.toml`.
pub fn path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("sts")
        .join("state.toml")
}

/// A missing or unreadable file means nothing is remembered yet.
pub fn load() -> State {
    fs::read_to_string(path())
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Applies `update` and saves the result. A failure only warns, because the command already did its work.
pub fn remember(update: impl FnOnce(&mut State)) {
    let mut state = load();
    update(&mut state);
    if let Err(err) = save(&state) {
        eprintln!("warning: cannot save {}: {err:#}", path().display());
    }
}

fn save(state: &State) -> Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?
        .write_all(toml::to_string(state)?.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_omits_unset_values() {
        let state = State {
            bluetooth: BluetoothState {
                device: Some("AA:BB:CC:DD:EE:FF".to_string()),
            },
            sonar: SonarState::default(),
        };
        let text = toml::to_string(&state).unwrap();
        assert!(!text.contains("host_url"));
        assert_eq!(toml::from_str::<State>(&text).unwrap(), state);
    }

    #[test]
    fn tolerates_unknown_and_missing_keys() {
        let state: State = toml::from_str("[sonar]\nhost_url = \"https://s\"\nold = 1\n").unwrap();
        assert_eq!(state.sonar.host_url.as_deref(), Some("https://s"));
        assert!(state.bluetooth.device.is_none());
    }
}

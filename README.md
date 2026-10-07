# sts

`sts` is a CLI for day-to-day company work on Ubuntu and Fedora.

## Quickstart

```sh
cargo install --path . --locked   # installs ~/.cargo/bin/sts
sts doctor                        # checks tools and prints install commands
sts config init                   # creates ~/.config/sts/config.toml
sts --help
```

For zsh completion, run `sts completions zsh > ~/.zfunc/_sts` and add `fpath=(~/.zfunc $fpath)` before `compinit` in `~/.zshrc`.

## Commands

| Command | What it does |
| --- | --- |
| `sts gh clone-org [ORG]` | It clones every repository of an organization in parallel and pulls the ones already cloned. |
| `sts ssm connect` | It lets you pick an EC2 instance and opens an SSM shell on it. |
| `sts ssm enable` | It attaches `AmazonSSMManagedInstanceCore` to an instance role. It writes to IAM, so it asks first. |
| `sts sonar scan [PATH]` | It asks for the server URL, project key and token, then runs the SonarQube scanner in a container. |
| `sts gnome workspaces [NAMES]` | It sets the GNOME workspace names. |
| `sts bluetooth pair [ADDRESS]` | It pairs a device by address, then trusts and connects it. It runs `sudo btmgmt`. Without an address, it asks for one. |
| `sts bluetooth connect [ADDRESS]` | It connects a paired device. Without an address, it uses the last device or lets you pick one. |
| `sts doctor` | It checks the required tools and prints install commands for your distro. |

`sts` calls the official CLIs, such as `gh`, `aws` and `docker`, so it reuses their logins and SSO sessions. Every command has `--help`. `sts bt` is short for `sts bluetooth`.

## Bluetooth

Some headsets never answer the laptop's discovery scan, but they still accept a direct connection. `sts bt pair` pairs by address, so it does not need the scan. Put the device in pairing mode first. `sts` remembers the address, so the next `pair` prefills it and `connect` reuses it.

## Config

Each value comes from the first source that sets it: a flag, an environment variable, `~/.config/sts/config.toml`, then a built-in default. `STS_CONFIG` points `sts` at another file.

| Config key | Flag | Environment variable |
| --- | --- | --- |
| `aws.region` | `-r, --region` | `AWS_REGION` |
| `aws.profile` | `-p, --profile` | `AWS_PROFILE` |
| `github.org` | `[ORG]` | |
| `sonar.host_url` | `--host-url` | `SONAR_HOST_URL` |
| `sonar.image` | `--image` | `SONAR_SCANNER_IMAGE` |
| `sonar.engine` | `--engine` | |
| `gnome.workspaces` | `[NAMES]` | |

`sonar scan` prefills its prompts from these values. It also prefills the last URL used, which takes priority over the config. Without a terminal, it skips the prompts, neither uses nor saves the last URL and requires `--project-key`.

`sts` keeps remembered values, such as the last Bluetooth device and SonarQube URL, in `~/.local/state/sts/state.toml`. It never stores tokens there. Delete the file to forget them.

This repository is public, so company values such as server URLs belong in the config file. Secrets belong in neither place. The SonarQube token comes from `SONAR_TOKEN` or a hidden prompt.

## Ubuntu and Fedora

- `sts doctor` prints `apt` hints on Ubuntu and `dnf` hints on Fedora.
- On Fedora, `sonar scan` uses Podman with `--userns=keep-id` and an SELinux `:z` mount. It refuses to mount `/` or `$HOME`.
- On Ubuntu, install Rust with rustup, because apt ships an old `rustc`.

## Development

```sh
just ci           # runs fmt, clippy and tests, as CI does
just run doctor   # runs sts from source
```

CI runs the same checks plus `cargo audit` on pushes to `main` and on pull requests. It also runs `cargo audit` weekly. Each command group lives in `src/commands/`.

## Roadmap

- `sts vpn` will manage the company FortiClient SSL VPN.

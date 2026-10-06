use std::process::Command;

use anyhow::{Context, Result, bail};

/// Runs `cmd`, returns its stdout, and fails with its stderr.
pub fn capture(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !out.status.success() {
        bail!(
            "`{}` failed: {}",
            describe(cmd),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8(out.stdout)
        .with_context(|| format!("`{}` printed invalid UTF-8", describe(cmd)))
}

/// Runs `cmd` attached to the terminal and fails on a non-zero exit.
pub fn status(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !status.success() {
        bail!("`{}` exited with {status}", describe(cmd));
    }
    Ok(())
}

fn describe(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

//! Pre/post apply hook runner. Both branches of `cmd_apply` shared this
//! boilerplate verbatim; only the hook field name and the user-facing label
//! changed. Non-zero exit warns but does not fail the apply.

use std::process::Command;

use anyhow::{Context, Result};

pub(super) fn run_apply_hook(
    label: &str,
    cmd: &str,
    target: &str,
    monitor: &str,
    integration: &str,
) -> Result<()> {
    if cmd.is_empty() {
        log::debug!("{label} not set, skipping");
        return Ok(());
    }
    log::info!("running {label}");
    let status = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .env("WALLRACK_WALLPAPER", target)
        .env("WALLRACK_MONITOR", monitor)
        .env("WALLRACK_INTEGRATION", integration)
        .status()
        .with_context(|| format!("spawn {label} `{cmd}`"))?;
    if !status.success() {
        log::warn!("{label} exited with {status}");
    }
    Ok(())
}

//! Backend command execution — the platform-specific glue that talks to the
//! actual wallpaper daemon and compositor. Each integration carries a
//! [`BackendConfig`] in the user's `config.toml`; this module substitutes
//! placeholders in the configured templates and shells them out.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::config::BackendConfig;

/// Substitute `{{name}}` placeholders in `template` with the given values.
/// Unknown placeholders are left as-is so the caller sees them in error output.
pub fn substitute(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn run_shell(cmd: &str) -> Result<std::process::Output> {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .with_context(|| format!("spawn `sh -c {cmd}`"))
}

/// Run the configured `apply_cmd` blocking.
pub fn run_apply(cfg: &BackendConfig, vars: &[(&str, &str)]) -> Result<()> {
    let template = cfg
        .apply_cmd
        .as_deref()
        .ok_or_else(|| anyhow!("apply_cmd not configured for this integration"))?;
    let cmd = substitute(template, vars);
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()
        .with_context(|| format!("spawn apply `{cmd}`"))?;
    if !status.success() {
        return Err(anyhow!("apply cmd `{cmd}` exited with {status}"));
    }
    Ok(())
}

/// Run the configured `apply_cmd` detached. Useful when called from the rofi
/// picker — rofi waits for the calling script to exit, so a long-running
/// `linux-wallpaperengine` would block the UI. Returns immediately.
pub fn run_apply_detached(cfg: &BackendConfig, vars: &[(&str, &str)]) -> Result<()> {
    let template = cfg
        .apply_cmd
        .as_deref()
        .ok_or_else(|| anyhow!("apply_cmd not configured for this integration"))?;
    let cmd = substitute(template, vars);
    use std::process::Stdio;
    // setsid puts the child in a new session so it survives the parent.
    Command::new("setsid")
        .arg("sh")
        .arg("-c")
        .arg(&cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn detached `{cmd}`"))?;
    Ok(())
}

/// Run the configured `monitors_cmd`, returning monitor names. The command
/// must print one name per line. Empty lines are skipped.
pub fn run_monitors(cfg: &BackendConfig) -> Result<Vec<String>> {
    let template = cfg
        .monitors_cmd
        .as_deref()
        .ok_or_else(|| anyhow!("monitors_cmd not configured for this integration"))?;
    let output = run_shell(template)?;
    if !output.status.success() {
        return Err(anyhow!(
            "monitors_cmd failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Run the optional `current_image_cmd`. Expects `<monitor>\t<path>` lines.
/// Returns an empty map when unset or when the command fails — this feature
/// is purely cosmetic (thumbnails in the monitor picker).
pub fn run_current_image(cfg: &BackendConfig) -> HashMap<String, PathBuf> {
    let Some(template) = cfg.current_image_cmd.as_deref() else {
        return HashMap::new();
    };
    let Ok(output) = run_shell(template) else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some((mon, path)) = line.split_once('\t') {
            let mon = mon.trim();
            let path = path.trim();
            if !mon.is_empty() && !path.is_empty() {
                map.insert(mon.to_string(), PathBuf::from(path));
            }
        }
    }
    map
}

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, anyhow};

use crate::applied::Applied;
use crate::config::Config;
use crate::entry::Entry;
use crate::integrations::{self, wallpaper_engine};
use crate::paths::Paths;

use super::super::hooks::run_apply_hook;
use super::super::state_helpers::resolve_integration;

pub(in crate::cli) fn run(
    paths: &Paths,
    config: &Config,
    integration: Option<&str>,
    monitor: &str,
    target: &str,
) -> Result<ExitCode> {
    let integration = resolve_integration(paths, integration)?;
    let integ = integrations::by_name(&integration)?;
    let entry = resolve_entry(&integration, target, paths)?;
    run_apply_hook(
        "pre_apply_hook",
        &config.hooks.pre_apply_hook,
        target,
        monitor,
        &integration,
    )?;
    integ.apply(&entry, monitor, paths, config)?;
    // WE writes its own applied entry inside `apply` so the launch_for batch
    // can read the full per-monitor set in one go. Image integrations don't,
    // so we record it here. The target stored is integration-specific: WE
    // uses the workshop id; image integrations use the entry id (image path).
    if integration != wallpaper_engine::NAME {
        let applied = Applied::open(paths.store())?;
        applied.set(monitor, &integration, target)?;
    }
    run_apply_hook(
        "post_apply_hook",
        &config.hooks.post_apply_hook,
        target,
        monitor,
        &integration,
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Look up the entry from the integration's index, falling back to a
/// synthesized image entry for the wallpaper/we_image integrations when the
/// target is an on-disk file that hasn't been indexed yet. The fallback is
/// what makes the booru-download → apply flow work without a re-index, and
/// what lets `applied restore` resurrect a pre-deleted entry by path.
pub(crate) fn resolve_entry(integration: &str, target: &str, paths: &Paths) -> Result<Entry> {
    let integ = integrations::by_name(integration)?;
    if let Ok(index) = integ.read_index(paths) {
        if let Some(e) = index.entries.iter().find(|e| e.id() == target).cloned() {
            return Ok(e);
        }
    }
    let p = PathBuf::from(target);
    let title = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    match integration {
        "wallpaper" if p.is_file() => Ok(Entry::Image {
            id: target.to_string(),
            title,
            source: p.clone(),
            thumb: PathBuf::new(),
            tags: Vec::new(),
            rating: String::new(),
            subfolder: String::new(),
            root: p
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
        }),
        "we_image" if p.is_file() => Ok(Entry::WeImage {
            id: target.to_string(),
            title,
            source: p.clone(),
            thumb: PathBuf::new(),
            tags: Vec::new(),
            rating: String::new(),
            subfolder: String::new(),
            workshop_id: String::new(),
            project_root: p
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
        }),
        _ => Err(anyhow!("entry not in index: {target}")),
    }
}

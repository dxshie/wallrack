use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, anyhow};

use crate::config::Config;
use crate::entry::Entry;
use crate::integrations;
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
    let index = integ.read_index(paths)?;
    let entry = match index.entries.iter().find(|e| e.id() == target).cloned() {
        Some(e) => e,
        None => {
            // For image integrations, allow applying an extant file even when
            // it isn't in the index yet. This is how the booru flow gets a
            // freshly-downloaded image onto a monitor without forcing a
            // re-index between download and apply.
            let p = PathBuf::from(target);
            let title = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("image")
                .to_string();
            match integration.as_str() {
                "wallpaper" if p.is_file() => Entry::Image {
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
                },
                "we_image" if p.is_file() => Entry::WeImage {
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
                },
                _ => return Err(anyhow!("entry not in index: {target}")),
            }
        }
    };
    run_apply_hook(
        "pre_apply_hook",
        &config.hooks.pre_apply_hook,
        target,
        monitor,
        &integration,
    )?;
    integ.apply(&entry, monitor, paths, config)?;
    run_apply_hook(
        "post_apply_hook",
        &config.hooks.post_apply_hook,
        target,
        monitor,
        &integration,
    )?;
    Ok(ExitCode::SUCCESS)
}

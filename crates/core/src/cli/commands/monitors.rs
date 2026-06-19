use std::collections::HashMap;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::entry::Index;
use crate::integrations::{self, wallpaper_engine};
use crate::output::{Action, Format, Row, ViewHints, write_rows};
use crate::paths::Paths;

use super::super::state_helpers::resolve_integration;

pub(in crate::cli) fn run(
    paths: &Paths,
    config: &Config,
    integration: Option<&str>,
    target: Option<&str>,
    format: Format,
) -> Result<ExitCode> {
    let integration = resolve_integration(paths, integration)?;

    let integ = integrations::by_name(&integration)?;
    let merged = integ.merged_backend(config);
    let monitors = integrations::backend::run_monitors(&merged)
        .with_context(|| format!("list monitors for {integration}"))?;
    let thumbs = current_thumbs(&integration, paths, config);

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    match format {
        Format::Json => {
            let list: Vec<_> = monitors
                .iter()
                .map(|m| {
                    let icon = thumbs.get(m);
                    serde_json::json!({ "name": m, "current_icon": icon })
                })
                .collect();
            serde_json::to_writer(&mut out, &list)?;
        }
        Format::Rofi => {
            // Rofi uses $selection (the label) as the monitor name and
            // ROFI_INFO (the info field) as the target to apply — so target
            // must be in info here. The picker shell reads ROFI_INFO
            // verbatim as the apply target; Action::Raw keeps the wire
            // bytes identical to the pre-typed-action era.
            let rows: Vec<Row<'_>> = monitors
                .iter()
                .map(|m| Row::Control {
                    label: m.clone(),
                    action: Action::Raw { value: target.unwrap_or_default().to_string() },
                    icon: thumbs.get(m).cloned(),
                })
                .collect();
            let hints = ViewHints {
                prompt: "Monitor".to_string(),
                ..ViewHints::default()
            };
            write_rows(&mut out, &rows, &hints, format)?;
        }
        Format::Walker | Format::Fuzzel | Format::Wofi => {
            // dmenu pickers (walker/fuzzel/wofi) return the payload column of
            // the selected row, so put the monitor name there. The target is
            // already known by the caller and does not need to be round-tripped
            // through the picker. Action::Raw tunnels the bare name.
            let rows: Vec<Row<'_>> = monitors
                .iter()
                .map(|m| Row::Control {
                    label: m.clone(),
                    action: Action::Raw { value: m.clone() },
                    icon: thumbs.get(m).cloned(),
                })
                .collect();
            let hints = ViewHints {
                prompt: "Monitor".to_string(),
                ..ViewHints::default()
            };
            write_rows(&mut out, &rows, &hints, format)?;
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Per-monitor current-wallpaper thumbnails. WE tracks its own state
/// (linux-wallpaperengine has no introspection), the other integrations rely
/// on the backend's optional `current_image_cmd`.
fn current_thumbs(
    integration: &str,
    paths: &Paths,
    config: &Config,
) -> HashMap<String, PathBuf> {
    if integration == "we" {
        let state = wallpaper_engine::read_monitor_state(paths);
        if state.is_empty() {
            return HashMap::new();
        }
        let idx_path = paths.index_file("we");
        let Ok(raw) = std::fs::read_to_string(&idx_path) else {
            return HashMap::new();
        };
        let Ok(idx) = serde_json::from_str::<Index>(&raw) else {
            return HashMap::new();
        };
        let by_workshop: HashMap<String, PathBuf> = idx
            .entries
            .into_iter()
            .filter_map(|e| match e {
                crate::entry::Entry::Project { workshop_id, thumb, .. } => Some((workshop_id, thumb)),
                _ => None,
            })
            .collect();
        return state
            .into_iter()
            .filter_map(|(mon, wid)| by_workshop.get(&wid).cloned().map(|t| (mon, t)))
            .collect();
    }
    let Ok(integ) = integrations::by_name(integration) else {
        return HashMap::new();
    };
    integrations::backend::run_current_image(&integ.merged_backend(config))
}

//! `wallrack tags` — list distinct tags across the active integration's
//! index, each row carrying a sample thumbnail.

use std::collections::BTreeMap;
use std::io::{self, BufWriter};
use std::process::ExitCode;

use anyhow::Result;

use crate::integrations;
use crate::output::{Format, Row, ViewHints, write_rows};
use crate::paths::Paths;

use super::super::state_helpers::resolve_integration;

pub(in crate::cli) fn run(
    paths: &Paths,
    integration: Option<&str>,
    format: Format,
) -> Result<ExitCode> {
    let integration = resolve_integration(paths, integration)?;
    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;

    // Map each tag to the first entry we see whose thumbnail exists on disk —
    // that gives rofi something to render next to the tag label. Entries
    // without a usable thumb still contribute the tag itself.
    let mut tag_thumb: BTreeMap<&str, Option<&std::path::Path>> = BTreeMap::new();
    for e in &index.entries {
        let thumb: Option<&std::path::Path> = (!e.thumb.as_os_str().is_empty())
            .then(|| e.thumb.as_path());
        for t in &e.tags {
            if t.is_empty() {
                continue;
            }
            let slot = tag_thumb.entry(t.as_str()).or_insert(None);
            if slot.is_none() {
                if let Some(p) = thumb {
                    if p.exists() {
                        *slot = Some(p);
                    }
                }
            }
        }
    }

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    match format {
        Format::Json => {
            let list: Vec<&str> = tag_thumb.keys().copied().collect();
            serde_json::to_writer(&mut out, &list)?;
        }
        Format::Rofi | Format::Walker | Format::Wofi | Format::Fuzzel => {
            // Header + "All tags" reset row + one row per tag.
            let mut rows: Vec<Row<'_>> = Vec::new();
            rows.push(Row::Control {
                label: "All tags".to_string(),
                info: "tag:".to_string(),
                icon: None,
            });
            for (t, thumb) in &tag_thumb {
                rows.push(Row::Control {
                    label: t.to_string(),
                    info: format!("tag:{t}"),
                    icon: thumb.map(|p| p.to_path_buf()),
                });
            }
            let hints = ViewHints {
                prompt: "Filter by Tag".to_string(),
                message: "Select a tag — Alt+2 to cancel".to_string(),
                use_hot_keys: true,
                allow_custom: false,
                filter: String::new(),
            };
            write_rows(&mut out, &rows, &hints, format)?;
        }
    }
    use std::io::Write;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

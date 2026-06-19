use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::ValueEnum;

use crate::entry::Entry;

pub mod action;
mod dmenu;
pub mod rofi;

pub use action::Action;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Format {
    /// Rofi script-mode protocol (null-separated rows with icon/info hints).
    Rofi,
    /// Walker dmenu TSV: `LABEL\tICON\tINFO` per line.
    Walker,
    /// Wofi dmenu with `img:` prefix; routing payload tacked on after U+001F.
    Wofi,
    /// Fuzzel — TSV-compatible with walker, parsed by the bundled fuzzel
    /// picker shim before being passed to fuzzel's native dmenu mode.
    Fuzzel,
    /// JSON array of entries — for any other picker / programmatic use.
    Json,
}

/// Hints rendered alongside entries — used for the rofi header (prompt,
/// message, hotkeys). Other formats ignore these.
#[derive(Debug, Default)]
pub struct ViewHints {
    pub prompt: String,
    pub message: String,
    pub use_hot_keys: bool,
    /// Set true to explicitly enable free-form input on a script-mode view
    /// (rofi `no-custom: false`). Needed for the booru search prompt where
    /// the user's typed query becomes the row label.
    pub allow_custom: bool,
    /// Pre-populate the picker's input field. Empty means "leave blank".
    /// Used to prime the booru search prompt with the active query so the
    /// user can edit it instead of retyping. Rofi-only — other pickers
    /// don't expose an equivalent.
    pub filter: String,
}

/// A row to emit. Either a real entry or a synthetic control row
/// (e.g. "← Back", "All tags"). Control rows carry only the routing action.
pub enum Row<'a> {
    Entry {
        entry: &'a Entry,
        favorite: bool,
        /// Override the label shown in the picker (overrides the entry title).
        label: Option<String>,
        /// Override the routing action. `None` defaults to
        /// `Action::ApplyImage { id: entry.id }` for image-based integrations.
        action: Option<Action>,
    },
    Control {
        label: String,
        action: Action,
        /// Optional thumbnail to render alongside the row (e.g. a sample image
        /// for a tag in the tag-picker view). Ignored when empty.
        icon: Option<PathBuf>,
    },
}

/// Render `rows` using the chosen format. `hints` is only consulted for rofi.
pub fn write_rows<W: Write>(
    w: &mut W,
    rows: &[Row<'_>],
    hints: &ViewHints,
    format: Format,
) -> Result<()> {
    match format {
        Format::Rofi => rofi::write(w, rows, hints),
        Format::Walker | Format::Fuzzel => dmenu::write(w, rows, hints, &dmenu::WALKER),
        Format::Wofi => dmenu::write(w, rows, hints, &dmenu::WOFI),
        Format::Json => write_json(w, rows),
    }
}

/// Common display/icon/payload triple extracted from any `Row`. The dmenu
/// dialects and the rofi renderer both build their wire format from these.
pub(crate) struct RowParts {
    pub display: String,
    pub icon: String,
    pub payload: String,
}

pub(crate) fn row_parts(row: &Row<'_>) -> RowParts {
    match row {
        Row::Entry {
            entry,
            favorite,
            label,
            action,
        } => {
            let star = if *favorite { "\u{2605} " } else { "" };
            let display = match label {
                Some(custom) => format!("{star}{custom}"),
                None => format!("{star}{} - {}", entry.title(), entry.id()),
            };
            let icon = entry.thumb().to_string_lossy().into_owned();
            let payload = action
                .as_ref()
                .map(Action::to_legacy_string)
                .unwrap_or_else(|| {
                    Action::ApplyImage {
                        id: entry.id().to_string(),
                    }
                    .to_legacy_string()
                });
            RowParts {
                display,
                icon,
                payload,
            }
        }
        Row::Control {
            label,
            action,
            icon,
        } => RowParts {
            display: label.clone(),
            icon: icon
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            payload: action.to_legacy_string(),
        },
    }
}

fn write_json<W: Write>(w: &mut W, rows: &[Row<'_>]) -> Result<()> {
    use serde_json::{Value, json};
    let arr: Vec<Value> = rows
        .iter()
        .map(|row| match row {
            Row::Entry {
                entry,
                favorite,
                label,
                action,
            } => {
                // Default action mirrors the dmenu default: ApplyImage with
                // the entry id. JSON consumers get the structured form, plus
                // the legacy `info` string for backward-compat parsers.
                let resolved = action.clone().unwrap_or_else(|| Action::ApplyImage {
                    id: entry.id().to_string(),
                });
                json!({
                    "integration": entry.integration(),
                    "id": entry.id(),
                    "title": label.clone().unwrap_or_else(|| entry.title().to_string()),
                    "source": entry.source(),
                    "thumb": entry.thumb(),
                    "tags": entry.tags(),
                    "rating": entry.rating(),
                    "workshop_id": entry.workshop_id(),
                    "subfolder": entry.subfolder().unwrap_or(""),
                    "favorite": favorite,
                    "info": resolved.to_legacy_string(),
                    "action": resolved,
                    "entry": entry,
                })
            }
            Row::Control {
                label,
                action,
                icon,
            } => json!({
                "control": true,
                "label": label,
                "info": action.to_legacy_string(),
                "action": action,
                "icon": icon,
            }),
        })
        .collect();
    serde_json::to_writer(w, &arr)?;
    Ok(())
}

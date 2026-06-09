use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::ValueEnum;

use crate::entry::Entry;

pub mod fuzzel;
pub mod rofi;
pub mod walker;
pub mod wofi;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Format {
    /// Rofi script-mode protocol (null-separated rows with icon/info hints).
    Rofi,
    /// Walker dmenu TSV: `LABEL\tICON\tINFO` per line.
    Walker,
    /// Wofi dmenu with `img:` prefix; routing payload tacked on after U+001F.
    Wofi,
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
}

/// A row to emit. Either a real entry or a synthetic control row
/// (e.g. "← Back", "All tags"). Control rows carry only the `info` hint.
pub enum Row<'a> {
    Entry {
        entry: &'a Entry,
        favorite: bool,
        /// Override the label shown in the picker (overrides the entry title).
        label: Option<String>,
        /// Override the info payload (defaults to the entry id).
        info: Option<String>,
    },
    Control {
        label: String,
        info: String,
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
        Format::Walker => walker::write(w, rows, hints),
        Format::Wofi => wofi::write(w, rows, hints),
        Format::Fuzzel => fuzzel::write(w, rows, hints),
        Format::Json => write_json(w, rows),
    }
}

fn write_json<W: Write>(w: &mut W, rows: &[Row<'_>]) -> Result<()> {
    use serde_json::{json, Value};
    let arr: Vec<Value> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Entry {
                entry,
                favorite,
                label,
                info,
            } => Some(json!({
                "integration": entry.integration,
                "id": entry.id,
                "title": label.clone().unwrap_or_else(|| entry.title.clone()),
                "source": entry.source,
                "thumb": entry.thumb,
                "tags": entry.tags,
                "rating": entry.rating,
                "workshop_id": entry.workshop_id,
                "subfolder": entry.subfolder,
                "favorite": favorite,
                "info": info,
            })),
            Row::Control { label, info, icon } => Some(json!({
                "control": true,
                "label": label,
                "info": info,
                "icon": icon,
            })),
        })
        .collect();
    serde_json::to_writer(w, &arr)?;
    Ok(())
}

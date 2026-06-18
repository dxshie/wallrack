use std::io::Write;

use anyhow::Result;

use super::{Row, ViewHints};

// U+001F (unit separator). Wofi has no `info` channel like rofi — the
// selected line is what comes back on stdout. We append the routing payload
// after this separator so the wrapper can split it off. The separator
// renders as a zero-width glyph in most fonts.
const UNIT_SEP: u8 = 0x1f;

/// Write rows in wofi dmenu format. Wofi is invoked with `--dmenu
/// --allow-images`; the `img:` prefix attaches a thumbnail to the row.
/// Hints are dropped — wofi doesn't have header rows, so the wrapper sets
/// prompt/message via the wofi command line.
pub fn write<W: Write>(w: &mut W, rows: &[Row<'_>], _hints: &ViewHints) -> Result<()> {
    for row in rows {
        match row {
            Row::Entry { entry, favorite, label, info } => {
                let star = if *favorite { "\u{2605} " } else { "" };
                let display = match label {
                    Some(custom) => format!("{star}{custom}"),
                    None => format!("{star}{} - {}", entry.title, entry.id),
                };
                let icon = entry.thumb.to_string_lossy();
                let payload = info
                    .clone()
                    .unwrap_or_else(|| format!("image:{}", entry.id));
                emit(w, &display, &icon, &payload)?;
            }
            Row::Control { label, info, icon } => {
                let icon_str = icon
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                emit(w, label, &icon_str, info)?;
            }
        }
    }
    Ok(())
}

fn emit<W: Write>(w: &mut W, display: &str, icon: &str, payload: &str) -> Result<()> {
    // Strip stray separators / newlines so they can't desync the wrapper.
    let display = sanitize(display);
    let payload = sanitize(payload);
    if !icon.is_empty() {
        write!(w, "img:{icon}:text:")?;
    }
    w.write_all(display.as_bytes())?;
    w.write_all(&[UNIT_SEP])?;
    w.write_all(payload.as_bytes())?;
    writeln!(w)?;
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' | '\r' | '\u{1f}' => ' ',
            other => other,
        })
        .collect()
}

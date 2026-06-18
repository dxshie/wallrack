use std::io::Write;

use anyhow::Result;

use super::{Row, ViewHints};

/// Write rows in walker dmenu TSV format: `LABEL\tICON\tINFO\n`. Walker's
/// dmenu mode echoes the selected line as-is on stdout; the wrapper splits
/// on tab to recover the icon and routing payload. Walker can render the
/// icon column natively when its dmenu module is configured with an icon
/// field (see the reference picker script).
///
/// Hints (prompt/message) are dropped here — walker takes those via CLI
/// flags / config, not via the input stream.
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
    // Tabs and newlines in any field would desync the TSV; collapse to spaces.
    let display = sanitize(display);
    let icon = sanitize(icon);
    let payload = sanitize(payload);
    write!(w, "{display}\t{icon}\t{payload}")?;
    writeln!(w)?;
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect()
}

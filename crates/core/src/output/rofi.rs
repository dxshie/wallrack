use std::io::Write;

use anyhow::Result;

use super::{Row, ViewHints};

// Rofi script-mode field separators. See `man rofi-script`.
const NUL: u8 = 0;
const SEP: u8 = 0x1f; // unit separator between key/value pairs

pub fn write<W: Write>(w: &mut W, rows: &[Row<'_>], hints: &ViewHints) -> Result<()> {
    // Header rows must come BEFORE any entries. Each starts with NUL.
    if !hints.prompt.is_empty() {
        w.write_all(&[NUL])?;
        write!(w, "prompt")?;
        w.write_all(&[SEP])?;
        w.write_all(hints.prompt.as_bytes())?;
        writeln!(w)?;
    }
    if hints.use_hot_keys {
        w.write_all(&[NUL])?;
        write!(w, "use-hot-keys")?;
        w.write_all(&[SEP])?;
        write!(w, "true")?;
        writeln!(w)?;
    }
    if !hints.message.is_empty() {
        w.write_all(&[NUL])?;
        write!(w, "message")?;
        w.write_all(&[SEP])?;
        w.write_all(hints.message.as_bytes())?;
        writeln!(w)?;
    }
    if hints.allow_custom {
        // Rofi's script-mode default is `no-custom: true`. Flip it off so
        // typed input passes through as `$selection` on Enter.
        w.write_all(&[NUL])?;
        write!(w, "no-custom")?;
        w.write_all(&[SEP])?;
        write!(w, "false")?;
        writeln!(w)?;
    }
    if !hints.filter.is_empty() {
        w.write_all(&[NUL])?;
        write!(w, "filter")?;
        w.write_all(&[SEP])?;
        w.write_all(hints.filter.as_bytes())?;
        writeln!(w)?;
    }

    for row in rows {
        match row {
            Row::Entry { entry, favorite, label, action } => {
                let star = if *favorite { "★ " } else { "" };
                // When caller supplies a label, trust it verbatim (e.g. folder
                // grouped rows). Otherwise append `- <id>` so a shell using
                // text extraction can still recover the path.
                let line = match label {
                    Some(custom) => format!("{star}{custom}"),
                    None => format!("{star}{} - {}", entry.title, entry.id),
                };
                w.write_all(line.as_bytes())?;
                // Rofi metadata: one NUL between display text and the first
                // pair, then SEP between subsequent pairs. A NUL between pairs
                // hides everything after the first one (including `info`),
                // which is why folder rows never drilled in.
                let mut wrote_meta = false;
                if !entry.thumb.as_os_str().is_empty() {
                    w.write_all(&[NUL])?;
                    wrote_meta = true;
                    write!(w, "icon")?;
                    w.write_all(&[SEP])?;
                    w.write_all(entry.thumb.to_string_lossy().as_bytes())?;
                }
                // Emit `info` only when the caller asked for it. The shell
                // routes selections off this field — Action::to_legacy_string
                // produces the same magic-string forms the shells already parse.
                if let Some(act) = action {
                    w.write_all(&[if wrote_meta { SEP } else { NUL }])?;
                    write!(w, "info")?;
                    w.write_all(&[SEP])?;
                    w.write_all(act.to_legacy_string().as_bytes())?;
                }
                writeln!(w)?;
            }
            Row::Control { label, action, icon } => {
                w.write_all(label.as_bytes())?;
                let mut wrote_meta = false;
                if let Some(path) = icon {
                    if !path.as_os_str().is_empty() {
                        w.write_all(&[NUL])?;
                        wrote_meta = true;
                        write!(w, "icon")?;
                        w.write_all(&[SEP])?;
                        w.write_all(path.to_string_lossy().as_bytes())?;
                    }
                }
                w.write_all(&[if wrote_meta { SEP } else { NUL }])?;
                write!(w, "info")?;
                w.write_all(&[SEP])?;
                w.write_all(action.to_legacy_string().as_bytes())?;
                writeln!(w)?;
            }
        }
    }
    Ok(())
}

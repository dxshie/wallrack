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

    for row in rows {
        match row {
            Row::Entry { entry, favorite, label, info } => {
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
                // Emit `info` only when the caller asked for it. Defaulting
                // to entry.id breaks the shell's "no info ⇒ open monitor
                // picker" convention for top-level wallpapers.
                if let Some(info_str) = info {
                    w.write_all(&[if wrote_meta { SEP } else { NUL }])?;
                    write!(w, "info")?;
                    w.write_all(&[SEP])?;
                    w.write_all(info_str.as_bytes())?;
                }
                writeln!(w)?;
            }
            Row::Control { label, info, icon } => {
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
                w.write_all(info.as_bytes())?;
                writeln!(w)?;
            }
        }
    }
    Ok(())
}

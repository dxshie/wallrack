use std::io::Write;

use anyhow::Result;

use super::{Row, ViewHints};

// Raffi (https://github.com/chmouel/raffi) is a fuzzel-backed launcher driven
// by a YAML config. It has no dmenu / stdin protocol of its own, so this
// writer emits a full raffi config: a `version: 1` header followed by a
// `launchers:` map with one entry per row.
//
// Each launcher uses `binary: echo` and stuffs the routing payload into the
// single arg. With `raffi --print-only` the selected entry prints
// `echo <payload>` to stdout, which the wrapper script trims back to the
// payload. The same payload prefixes the rofi / wofi formats already use
// (`image:`, `folder:`, `back:`, `tag:`, `tagedit:*`, `noop:*`, `action:*`)
// work verbatim.
//
// ViewHints aren't exposed by raffi — there's no way to set fuzzel's prompt
// through the YAML — so the wrapper conveys mode/view by prepending control
// rows like the wofi script.

pub fn write<W: Write>(w: &mut W, rows: &[Row<'_>], _hints: &ViewHints) -> Result<()> {
    writeln!(w, "version: 1")?;
    writeln!(w, "general:")?;
    writeln!(w, "  ui_type: fuzzel")?;
    writeln!(w, "  max_history: 0")?;
    writeln!(w, "launchers:")?;

    for (idx, row) in rows.iter().enumerate() {
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
                emit(w, idx, &display, Some(&icon), &payload)?;
            }
            Row::Control { label, info, icon } => {
                let icon_str = icon
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let icon_ref = if icon_str.is_empty() { None } else { Some(icon_str.as_str()) };
                emit(w, idx, label, icon_ref, info)?;
            }
        }
    }
    Ok(())
}

fn emit<W: Write>(
    w: &mut W,
    idx: usize,
    display: &str,
    icon: Option<&str>,
    payload: &str,
) -> Result<()> {
    let display = sanitize(display);
    let payload = sanitize(payload);
    // Entry keys must be unique within the YAML map. Their textual value is
    // never user-visible (raffi shows `description`), so just number them.
    writeln!(w, "  entry_{idx}:")?;
    writeln!(w, "    binary: echo")?;
    writeln!(w, "    args: [{}]", yaml_quote(&payload))?;
    writeln!(w, "    description: {}", yaml_quote(&display))?;
    if let Some(path) = icon {
        let path = sanitize(path);
        if !path.is_empty() {
            writeln!(w, "    icon: {}", yaml_quote(&path))?;
        }
    }
    Ok(())
}

// Single-quoted YAML scalar: the only escape is `'` → `''`. Newlines and
// other control chars are already stripped upstream by `sanitize`.
fn yaml_quote(s: &str) -> String {
    let escaped = s.replace('\'', "''");
    format!("'{escaped}'")
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect()
}

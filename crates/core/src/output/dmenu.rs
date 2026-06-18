//! Shared dmenu-style row writer used by the walker, fuzzel, and wofi
//! formats. Each dialect differs only in how the icon attaches to the row
//! and which field separator delimits the routing payload — capture both
//! as a `Dialect` and the emit function is one place.

use std::io::Write;

use anyhow::Result;

use super::{Row, ViewHints};

/// How a dialect carries the row's icon.
pub(super) enum IconStyle {
    /// Walker / fuzzel: icon is a separate column between display and payload.
    Column,
    /// Wofi: icon is fused into the display column as `img:<icon>:text:<display>`.
    LabelPrefix,
}

pub(super) struct Dialect {
    pub icon_style: IconStyle,
    pub field_sep: u8,
    pub strip: &'static [char],
}

pub(super) const WALKER: Dialect = Dialect {
    icon_style: IconStyle::Column,
    field_sep: b'\t',
    strip: &['\n', '\r', '\t'],
};

pub(super) const WOFI: Dialect = Dialect {
    icon_style: IconStyle::LabelPrefix,
    field_sep: 0x1f,
    strip: &['\n', '\r', '\u{1f}'],
};

/// Hints (prompt/message) are dropped — these pickers configure them via
/// CLI flags, not the input stream.
pub(super) fn write<W: Write>(
    w: &mut W,
    rows: &[Row<'_>],
    _hints: &ViewHints,
    d: &Dialect,
) -> Result<()> {
    for row in rows {
        let parts = super::row_parts(row);
        emit(w, &parts.display, &parts.icon, &parts.payload, d)?;
    }
    Ok(())
}

fn emit<W: Write>(
    w: &mut W,
    display: &str,
    icon: &str,
    payload: &str,
    d: &Dialect,
) -> Result<()> {
    let display = sanitize(display, d.strip);
    let icon = sanitize(icon, d.strip);
    let payload = sanitize(payload, d.strip);
    match d.icon_style {
        IconStyle::Column => {
            write!(w, "{display}\t{icon}\t{payload}")?;
        }
        IconStyle::LabelPrefix => {
            if !icon.is_empty() {
                write!(w, "img:{icon}:text:")?;
            }
            w.write_all(display.as_bytes())?;
            w.write_all(&[d.field_sep])?;
            w.write_all(payload.as_bytes())?;
        }
    }
    writeln!(w)?;
    Ok(())
}

fn sanitize(s: &str, strip: &[char]) -> String {
    s.chars()
        .map(|c| if strip.contains(&c) { ' ' } else { c })
        .collect()
}

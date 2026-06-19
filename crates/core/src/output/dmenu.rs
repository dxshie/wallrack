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

fn emit<W: Write>(w: &mut W, display: &str, icon: &str, payload: &str, d: &Dialect) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::entry::Entry;
    use crate::output::{Action, Format, Row, ViewHints, write_rows};

    fn sample_entry() -> Entry {
        Entry::Image {
            id: "/wp/a.jpg".into(),
            title: "title\twith tab".into(),
            source: PathBuf::from("/wp/a.jpg"),
            thumb: PathBuf::from("/cache/a.jpg"),
            tags: vec![],
            rating: String::new(),
            subfolder: String::new(),
            root: PathBuf::from("/wp"),
        }
    }

    #[test]
    fn walker_emits_three_tab_separated_columns_per_row() {
        let entry = sample_entry();
        let rows = vec![Row::Entry {
            entry: &entry,
            favorite: false,
            label: None,
            action: None,
        }];
        let mut buf = Vec::new();
        write_rows(&mut buf, &rows, &ViewHints::default(), Format::Walker).unwrap();
        let line = std::str::from_utf8(&buf).unwrap().trim_end_matches('\n');
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3, "expected 3 tab-separated cols: {line:?}");
        // Tab in title is sanitized to space so it doesn't break the TSV.
        assert!(cols[0].contains("title with tab"));
        assert_eq!(cols[1], "/cache/a.jpg");
        assert_eq!(cols[2], "image:/wp/a.jpg");
    }

    #[test]
    fn wofi_uses_img_prefix_when_icon_present_and_unit_separator() {
        let entry = sample_entry();
        let rows = vec![Row::Entry {
            entry: &entry,
            favorite: true,
            label: None,
            action: None,
        }];
        let mut buf = Vec::new();
        write_rows(&mut buf, &rows, &ViewHints::default(), Format::Wofi).unwrap();
        let line = std::str::from_utf8(&buf).unwrap().trim_end_matches('\n');
        assert!(line.starts_with("img:/cache/a.jpg:text:"));
        // Favorite star is the leading display char after the prefix.
        let display = line.trim_start_matches("img:/cache/a.jpg:text:");
        assert!(display.starts_with("\u{2605}"));
        // Unit separator splits display from payload.
        assert!(line.contains(0x1f as char));
        assert!(line.ends_with("image:/wp/a.jpg"));
    }

    #[test]
    fn control_rows_carry_their_action_payload() {
        let rows = vec![Row::Control {
            label: "← Back".to_string(),
            action: Action::Back,
            icon: None,
        }];
        let mut buf = Vec::new();
        write_rows(&mut buf, &rows, &ViewHints::default(), Format::Walker).unwrap();
        let line = std::str::from_utf8(&buf).unwrap().trim_end_matches('\n');
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols[0], "← Back");
        assert_eq!(cols[1], "");
        assert_eq!(cols[2], "back:");
    }
}

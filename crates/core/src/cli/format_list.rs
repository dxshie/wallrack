//! Single helper for the "emit a flat string list as JSON or one-per-line"
//! pattern. Several commands (tags, favorites list, tag available, booru
//! sites) used to inline this match — now they all call here.

use std::io::Write;

use anyhow::Result;

use crate::output::Format;

pub(super) fn write_string_list<W: Write, S: AsRef<str>>(
    w: &mut W,
    items: &[S],
    format: Format,
) -> Result<()> {
    match format {
        Format::Json => {
            let raw: Vec<&str> = items.iter().map(|s| s.as_ref()).collect();
            serde_json::to_writer(w, &raw)?;
        }
        Format::Rofi | Format::Walker | Format::Wofi | Format::Fuzzel => {
            for item in items {
                writeln!(w, "{}", item.as_ref())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_emits_a_json_string_array() {
        let mut buf = Vec::new();
        write_string_list(&mut buf, &["a", "b", "c"], Format::Json).unwrap();
        let parsed: Vec<String> = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed, vec!["a", "b", "c"]);
    }

    #[test]
    fn rofi_emits_newline_separated_items() {
        let mut buf = Vec::new();
        write_string_list(&mut buf, &["alpha", "beta"], Format::Rofi).unwrap();
        assert_eq!(buf, b"alpha\nbeta\n");
    }

    #[test]
    fn empty_input_produces_empty_output_for_dmenu_dialects() {
        for fmt in [Format::Rofi, Format::Walker, Format::Wofi, Format::Fuzzel] {
            let mut buf = Vec::new();
            let items: [&str; 0] = [];
            write_string_list(&mut buf, &items, fmt).unwrap();
            assert!(buf.is_empty(), "expected empty buf for {fmt:?}");
        }
    }

    #[test]
    fn empty_input_produces_empty_array_for_json() {
        let mut buf = Vec::new();
        let items: [&str; 0] = [];
        write_string_list(&mut buf, &items, Format::Json).unwrap();
        assert_eq!(buf, b"[]");
    }
}

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

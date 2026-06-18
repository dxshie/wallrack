use std::io::Write;

use anyhow::Result;

use super::{Row, ViewHints};

/// Write rows in fuzzel dmenu TSV format: `LABEL\tICON\tINFO\n`.
///
/// The fuzzel picker script runs fuzzel in `--dmenu --index` mode and uses a
/// `fuzzel_pick` helper that reads this walker-compatible TSV, assembles the
/// native fuzzel dmenu `label\0icon\x1fpath\n` stream internally, and returns
/// the payload (third column) of the selected row. Emitting TSV here keeps
/// the format consistent with `--format=walker` and lets the picker use
/// `--format=fuzzel` throughout without any jq post-processing.
pub fn write<W: Write>(w: &mut W, rows: &[Row<'_>], hints: &ViewHints) -> Result<()> {
    super::walker::write(w, rows, hints)
}

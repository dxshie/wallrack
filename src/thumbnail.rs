use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;

/// Generate a square center-cropped thumbnail at `size`x`size`. JPEG output.
/// Skips work when the destination is newer than the source.
pub fn generate(src: &Path, dst: &Path, size: u32) -> Result<()> {
    if up_to_date(src, dst) {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let img = image::open(src)
        .with_context(|| format!("decode {}", src.display()))?;
    // Fill: scale so the shorter side matches `size`, then center-crop.
    let (w, h) = (img.width(), img.height());
    let short = w.min(h).max(1);
    let scale = size as f32 / short as f32;
    let new_w = ((w as f32 * scale).round() as u32).max(size);
    let new_h = ((h as f32 * scale).round() as u32).max(size);
    let resized = img.resize_exact(new_w, new_h, FilterType::Triangle);
    let cropped = resized.crop_imm(
        (resized.width() - size) / 2,
        (resized.height() - size) / 2,
        size,
        size,
    );
    cropped.save(dst)
        .with_context(|| format!("write thumb {}", dst.display()))?;
    Ok(())
}

fn up_to_date(src: &Path, dst: &Path) -> bool {
    let (Ok(s), Ok(d)) = (std::fs::metadata(src), std::fs::metadata(dst)) else {
        return false;
    };
    if d.len() == 0 {
        return false;
    }
    match (s.modified(), d.modified()) {
        (Ok(sm), Ok(dm)) => dm >= sm,
        _ => false,
    }
}

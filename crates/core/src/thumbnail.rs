use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, RgbImage};

/// Generate a square center-cropped thumbnail at `size`x`size`. The output
/// format is inferred from `dst`'s extension by the `image` crate — wallrack
/// writes `.png` (see `thumb_filename_for`) so fuzzel's libpng-only icon
/// renderer can display the result. Skips work when the destination is newer
/// than the source.
pub fn generate(src: &Path, dst: &Path, size: u32) -> Result<()> {
    if up_to_date(src, dst) {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let img = decode(src, size).with_context(|| format!("decode {}", src.display()))?;
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
    cropped
        .save(dst)
        .with_context(|| format!("write thumb {}", dst.display()))?;
    Ok(())
}

/// Decode `src`. For JPEGs we use jpeg-decoder's scale-on-decode (1/8, 1/4,
/// 1/2) so a 4K workshop wallpaper turns into ~512px before any per-pixel work
/// runs — roughly an order of magnitude less data through the resize.
fn decode(src: &Path, target: u32) -> Result<DynamicImage> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("jpg") | Some("jpeg")) {
        if let Some(img) = jpeg_scaled(src, target) {
            return Ok(img);
        }
        // Pixel formats we can't directly wrap (CMYK, L16, …) fall through.
    }
    image::open(src).context("image::open")
}

fn jpeg_scaled(src: &Path, target: u32) -> Option<DynamicImage> {
    use jpeg_decoder::{Decoder, PixelFormat};
    let file = File::open(src).ok()?;
    let mut decoder = Decoder::new(BufReader::new(file));
    decoder.read_info().ok()?;
    let t = u16::try_from(target).unwrap_or(u16::MAX);
    // scale() picks the smallest of {1/8, 1/4, 1/2, 1/1} whose decoded size is
    // ≥ target in at least one axis. Returns the resulting dimensions.
    let (w, h) = decoder.scale(t, t).ok()?;
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (w, h) = (u32::from(w), u32::from(h));
    match info.pixel_format {
        PixelFormat::RGB24 => RgbImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgb8),
        PixelFormat::L8 => GrayImage::from_raw(w, h, pixels).map(DynamicImage::ImageLuma8),
        // CMYK24 / L16 are rare for wallpapers; let image::open handle them.
        _ => None,
    }
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

//! "Normal" wallpaper integration — plain image files from user-configured
//! directories (`wallpaper.dirs` in config.toml). Drilling into subfolders is
//! supported: an image at `<root>/sub/foo.jpg` is grouped under a `sub/`
//! folder row in the picker.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use rayon::prelude::*;

use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::integrations::backend;
use crate::integrations::progress::Progress;
use crate::integrations::scan::walk_images;
use crate::integrations::{Integration, thumb_filename_for};
use crate::paths::Paths;
use crate::thumbnail;

pub const NAME: &str = "wallpaper";

pub struct WallpaperIntegration;

impl Integration for WallpaperIntegration {
    fn name(&self) -> &'static str { NAME }
    fn label(&self) -> &'static str { "Wallpapers" }

    fn index(&self, paths: &Paths, config: &Config) -> Result<Index> {
        paths.ensure_integration(NAME)?;
        let thumbs_dir = paths.thumbs_dir(NAME);
        let thumb_size = config.thumbnails.size;

        let mut sources: Vec<EntrySource> = Vec::new();
        for dir in config.wallpaper_dirs() {
            if !dir.is_dir() {
                log::warn!("wallpaper dir not found, skipping: {}", dir.display());
                continue;
            }
            log::info!("wallpaper: scanning {}", dir.display());
            collect_images(&dir, &mut sources);
        }

        let total = sources.len();
        log::info!("wallpaper: {total} images found, generating thumbnails...");

        let progress = Progress::new("wallpaper", total);
        let entries: Vec<Entry> = sources
            .par_iter()
            .filter_map(|src| {
                let entry = build_entry(src, &thumbs_dir, thumb_size).ok();
                progress.tick();
                entry
            })
            .collect();
        progress.finish();

        let index = Index { integration: NAME.to_string(), entries };
        crate::integrations::write_index(paths, &index)?;
        Ok(index)
    }

    fn apply(&self, entry: &Entry, monitor: &str, _paths: &Paths, config: &Config) -> Result<()> {
        if monitor.is_empty() {
            return Err(anyhow!("apply: no monitor given"));
        }
        let img = &entry.source;
        if !img.exists() {
            return Err(anyhow!("image does not exist: {}", img.display()));
        }
        let img_str = img.to_string_lossy();
        backend::run_apply(
            &self.merged_backend(config),
            &[("image", img_str.as_ref()), ("monitor", monitor)],
        )
    }

    fn watch_dirs(&self, config: &Config) -> Vec<PathBuf> {
        config.wallpaper_dirs().into_iter().filter(|d| d.is_dir()).collect()
    }

    fn backend<'a>(&self, config: &'a Config) -> &'a crate::config::BackendConfig {
        &config.wallpaper.backend
    }

    fn default_backend(&self) -> crate::config::BackendConfig {
        image_backend_defaults()
    }
}

/// Defaults shared by the two image-applying integrations.
pub(crate) fn image_backend_defaults() -> crate::config::BackendConfig {
    crate::config::BackendConfig {
        apply_cmd: Some(
            r#"awww img "{{image}}" --transition-type center -o "{{monitor}}""#.into(),
        ),
        monitors_cmd: Some(r#"hyprctl monitors | awk '/^Monitor / {print $2}'"#.into()),
        current_image_cmd: Some(
            r#"awww query | sed -nE 's/^[ :]*([^:]+):.*image: (.+)$/\1\t\2/p'"#.into(),
        ),
    }
}

#[derive(Debug)]
struct EntrySource {
    image: PathBuf,
    root: PathBuf,
}

fn collect_images(root: &Path, out: &mut Vec<EntrySource>) {
    for img in walk_images(root, false) {
        out.push(EntrySource { image: img, root: root.to_path_buf() });
    }
}

fn build_entry(src: &EntrySource, thumbs_dir: &Path, size: u32) -> Result<Entry> {
    let thumb_name = thumb_filename_for(&src.image);
    let thumb = thumbs_dir.join(&thumb_name);
    let _ = thumbnail::generate(&src.image, &thumb, size);

    let subfolder = src
        .image
        .parent()
        .and_then(|p| p.strip_prefix(&src.root).ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Root-level images keep their filename as the title; nested images take
    // the configured root's name (e.g. "Pictures") so grouped folder rows
    // display as "Pictures - anime" rather than the first image's stem.
    let title = if subfolder.is_empty() {
        src.image
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string()
    } else {
        src.root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("wallpapers")
            .to_string()
    };

    // workshop_id is overloaded as a grouping discriminator for plain
    // wallpapers — using the root dir path keeps two configured dirs that
    // share a subfolder name from colliding in the picker.
    let workshop_id = Some(src.root.to_string_lossy().to_string());

    Ok(Entry {
        integration: NAME.to_string(),
        id: src.image.to_string_lossy().to_string(),
        title,
        source: src.image.clone(),
        thumb,
        rating: String::new(),
        tags: Vec::new(),
        workshop_id,
        subfolder,
    })
}

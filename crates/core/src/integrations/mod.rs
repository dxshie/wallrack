use std::path::Path;

use anyhow::{Result, anyhow};

use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::paths::Paths;

pub mod backend;
pub mod booru;
pub mod progress;
pub mod scan;
pub mod wallpaper;
pub mod wallpaper_engine;
pub mod wallpaper_engine_image;

/// File extensions every image-scanning integration considers.
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp", "gif", "webp"];

/// Common surface every wallpaper backend implements.
///
/// `name()` is also the on-disk integration key — it shows up in cache paths
/// (`~/.cache/wallrack/<name>/`), favorites, and state. Don't rename casually.
pub trait Integration {
    fn name(&self) -> &'static str;

    /// User-facing label for the picker prompt.
    fn label(&self) -> &'static str { self.name() }

    /// Whether drilling into subfolders is meaningful for this integration.
    /// `false` for the WE integration: it applies the whole project, not
    /// individual files.
    fn supports_drill(&self) -> bool { true }

    /// Rebuild the index from disk. Writes the index to its cache file as a
    /// side-effect and returns the freshly built [`Index`].
    fn index(&self, paths: &Paths, config: &Config) -> Result<Index>;

    /// Read the cached index. Errors if not built yet. User tag overrides
    /// (`tags.json`) are merged into each entry's `tags` field so callers
    /// see the effective set rather than the raw indexed tags.
    fn read_index(&self, paths: &Paths) -> Result<Index> {
        let file = paths.index_file(self.name());
        if !file.exists() {
            return Err(anyhow!(
                "{} index not built — run `wallrack index --integration={}` first",
                self.name(), self.name()
            ));
        }
        let raw = std::fs::read_to_string(&file)?;
        let mut idx: Index = serde_json::from_str(&raw).map_err(|e| {
            anyhow!(
                "failed to parse {} index ({}) — if this index was built by an \
                 older wallrack version, run `wallrack index --integration={}` \
                 to rebuild it in the new format",
                self.name(), e, self.name(),
            )
        })?;
        if let Ok(overrides) = crate::tags::TagOverrides::load(&paths.tags_file()) {
            overrides.apply_to(&mut idx);
        }
        if let Ok(ratings) = crate::rating::RatingOverrides::load(&paths.rating_overrides_file()) {
            ratings.apply_to(&mut idx);
        }
        Ok(idx)
    }

    /// Apply the entry — actually set the wallpaper on the given monitor.
    fn apply(&self, entry: &Entry, monitor: &str, paths: &Paths, config: &Config) -> Result<()>;

    /// Directories to watch with `wallrack daemon` for auto-reindex.
    fn watch_dirs(&self, config: &Config) -> Vec<std::path::PathBuf>;

    /// Backend config for this integration. Used by the monitor picker to
    /// list monitors and discover currently-displayed wallpapers.
    fn backend<'a>(&self, config: &'a Config) -> &'a crate::config::BackendConfig;

    /// Per-integration backend defaults — used to fill in any field the user
    /// did not set in `config.toml`. Override any of these in the
    /// `[<integration>.backend]` section of `config.toml`.
    fn default_backend(&self) -> crate::config::BackendConfig {
        crate::config::BackendConfig::default()
    }

    /// User backend with defaults filled in.
    fn merged_backend(&self, config: &Config) -> crate::config::BackendConfig {
        let user = self.backend(config);
        let defaults = self.default_backend();
        crate::config::BackendConfig {
            apply_cmd: user.apply_cmd.clone().or(defaults.apply_cmd),
            monitors_cmd: user.monitors_cmd.clone().or(defaults.monitors_cmd),
            current_image_cmd: user.current_image_cmd.clone().or(defaults.current_image_cmd),
        }
    }
}

pub fn all() -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(wallpaper::WallpaperIntegration),
        Box::new(wallpaper_engine_image::WallpaperEngineImageIntegration),
        Box::new(wallpaper_engine::WallpaperEngineIntegration),
        Box::new(booru::BooruIntegration),
    ]
}

pub fn by_name(name: &str) -> Result<Box<dyn Integration>> {
    match name {
        "wallpaper" => Ok(Box::new(wallpaper::WallpaperIntegration)),
        "we_image" | "wallpaper_engine_image" => {
            Ok(Box::new(wallpaper_engine_image::WallpaperEngineImageIntegration))
        }
        "we" | "wallpaper_engine" => Ok(Box::new(wallpaper_engine::WallpaperEngineIntegration)),
        "booru" => Ok(Box::new(booru::BooruIntegration)),
        other => Err(anyhow!("unknown integration: {other}")),
    }
}

pub fn names() -> Vec<&'static str> {
    all().iter().map(|i| i.name()).collect()
}

/// Persist the index to disk.
pub fn write_index(paths: &Paths, index: &Index) -> Result<()> {
    paths.ensure_integration(&index.integration)?;
    let file = paths.index_file(&index.integration);
    let body = serde_json::to_vec_pretty(index)?;
    crate::paths::atomic_write(&file, &body)?;
    Ok(())
}

/// Helper: pick a stable thumbnail filename for an arbitrary source path.
pub fn thumb_filename_for(source: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(source.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hex = hex::encode(&digest[..8]);
    let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("img");
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("entry")
        .replace('/', "_");
    // PNG output — fuzzel's icon renderer only links libpng + libresvg, so
    // JPG/GIF thumbs would never render in dmenu mode. rofi/wofi/walker all
    // handle PNG too, so this is the lowest-common-denominator format.
    format!("{hex}_{stem}.{ext}.png")
}

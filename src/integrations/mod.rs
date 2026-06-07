use std::path::Path;

use anyhow::{Result, anyhow};

use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::paths::Paths;

pub mod wallpaper;
pub mod wallpaper_engine;

/// Common surface every wallpaper backend implements.
///
/// `name()` is also the on-disk integration key — it shows up in cache paths
/// (`~/.cache/wallrack/<name>/`), favorites, and state. Don't rename casually.
pub trait Integration {
    fn name(&self) -> &'static str;

    /// Rebuild the index from disk. Writes the index to its cache file as a
    /// side-effect and returns the freshly built [`Index`].
    fn index(&self, paths: &Paths, config: &Config) -> Result<Index>;

    /// Read the cached index. Errors if not built yet.
    fn read_index(&self, paths: &Paths) -> Result<Index> {
        let file = paths.index_file(self.name());
        if !file.exists() {
            return Err(anyhow!(
                "{} index not built — run `wallrack index --integration={}` first",
                self.name(), self.name()
            ));
        }
        let raw = std::fs::read_to_string(&file)?;
        let idx: Index = serde_json::from_str(&raw)?;
        Ok(idx)
    }

    /// Apply the entry — actually set the wallpaper on the given monitor.
    fn apply(&self, entry: &Entry, monitor: &str, paths: &Paths) -> Result<()>;

    /// Directories to watch with `wallrack daemon` for auto-reindex.
    fn watch_dirs(&self, config: &Config) -> Vec<std::path::PathBuf>;
}

pub fn all() -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(wallpaper::WallpaperIntegration),
        Box::new(wallpaper_engine::WallpaperEngineIntegration),
    ]
}

pub fn by_name(name: &str) -> Result<Box<dyn Integration>> {
    match name {
        "wallpaper" => Ok(Box::new(wallpaper::WallpaperIntegration)),
        "we" | "wallpaper_engine" => Ok(Box::new(wallpaper_engine::WallpaperEngineIntegration)),
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
    format!("{hex}_{stem}.{ext}.jpg")
}

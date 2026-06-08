use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{Paths, expand_home};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub thumbnails: Thumbnails,
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    #[serde(default)]
    pub wallpaper_engine_image: WallpaperEngineImageConfig,
    #[serde(default)]
    pub wallpaper_engine: WallpaperEngineConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thumbnails {
    #[serde(default = "default_thumb_size")]
    pub size: u32,
}

/// Per-integration commands. Templates use `{{image}}`, `{{monitor}}`,
/// `{{folder}}`, `{{workshop_id}}` placeholders — substituted as plain text
/// and passed to `sh -c`, so users are responsible for quoting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Sets a wallpaper. Receives `{{image}}` / `{{folder}}` / `{{workshop_id}}`
    /// and `{{monitor}}` depending on the integration.
    #[serde(default)]
    pub apply_cmd: Option<String>,
    /// Lists monitors — must print one monitor name per line.
    #[serde(default)]
    pub monitors_cmd: Option<String>,
    /// Optional. Prints currently-displayed wallpapers as
    /// `<monitor>\t<path>` lines. When unset, the monitor picker simply
    /// doesn't show current-wallpaper thumbs.
    #[serde(default)]
    pub current_image_cmd: Option<String>,
}

/// Plain wallpaper images from user-provided directories.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WallpaperConfig {
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub backend: BackendConfig,
}

/// Wallpaper Engine workshop folders scraped for image assets — applies them
/// like a normal image wallpaper (not via linux-wallpaperengine).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WallpaperEngineImageConfig {
    /// Workshop dir. When unset, falls back to `wallpaper_engine.workshop_dir`
    /// since the typical setup uses the same source for both.
    #[serde(default)]
    pub workshop_dir: Option<String>,
    #[serde(default)]
    pub backend: BackendConfig,
}

/// Wallpaper Engine projects, applied via linux-wallpaperengine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperEngineConfig {
    #[serde(default = "default_we_workshop_dir")]
    pub workshop_dir: String,
    #[serde(default)]
    pub backend: BackendConfig,
}

impl Default for WallpaperEngineConfig {
    fn default() -> Self {
        Self { workshop_dir: default_we_workshop_dir(), backend: BackendConfig::default() }
    }
}

impl Default for Thumbnails {
    fn default() -> Self {
        Self { size: default_thumb_size() }
    }
}

fn default_thumb_size() -> u32 { 256 }

fn default_we_workshop_dir() -> String {
    "~/.local/share/Steam/steamapps/workshop/content/431960".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            thumbnails: Thumbnails::default(),
            wallpaper: WallpaperConfig::default(),
            wallpaper_engine_image: WallpaperEngineImageConfig::default(),
            wallpaper_engine: WallpaperEngineConfig::default(),
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.config_file();
        if !file.exists() {
            paths.ensure_config()?;
            let default = Config::default();
            let body = toml::to_string_pretty(&default).context("serialize default config")?;
            fs::write(&file, body)
                .with_context(|| format!("write default config to {}", file.display()))?;
            return Ok(default);
        }
        let body = fs::read_to_string(&file)
            .with_context(|| format!("read config {}", file.display()))?;
        let cfg: Config = toml::from_str(&body)
            .with_context(|| format!("parse config {}", file.display()))?;
        Ok(cfg)
    }

    pub fn wallpaper_dirs(&self) -> Vec<PathBuf> {
        self.wallpaper.dirs.iter().map(|s| expand_home(s)).collect()
    }

    /// Workshop dir for the we_image integration. Falls back to the WE
    /// workshop dir when not explicitly set — the typical setup points both
    /// at the same Steam workshop content folder.
    pub fn we_image_workshop_dir(&self) -> PathBuf {
        match self.wallpaper_engine_image.workshop_dir.as_deref() {
            Some(d) => expand_home(d),
            None => self.we_workshop_dir(),
        }
    }

    pub fn we_workshop_dir(&self) -> PathBuf {
        expand_home(&self.wallpaper_engine.workshop_dir)
    }
}

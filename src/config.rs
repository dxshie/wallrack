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
    pub wallpaper_engine: WallpaperEngineConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thumbnails {
    #[serde(default = "default_thumb_size")]
    pub size: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WallpaperConfig {
    /// Plain wallpaper directories (no project.json).
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Optional steam workshop dir to also pull image-based wallpapers from.
    #[serde(default)]
    pub steam_workshop_dir: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WallpaperEngineConfig {
    /// Path to Steam Workshop dir holding linux-wallpaperengine projects.
    #[serde(default = "default_we_workshop_dir")]
    pub workshop_dir: String,
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
            wallpaper_engine: WallpaperEngineConfig {
                workshop_dir: default_we_workshop_dir(),
            },
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

    /// Workshop dir for the wallpaper integration. Falls back to the WE
    /// workshop dir when not explicitly set, since the typical setup uses
    /// the same source for both — WE projects are scanned for image
    /// wallpapers (wallpaper mode) AND for live wallpapers (WE mode).
    pub fn wallpaper_steam_dir(&self) -> Option<PathBuf> {
        match self.wallpaper.steam_workshop_dir.as_deref() {
            Some(d) => Some(expand_home(d)),
            None => Some(self.we_workshop_dir()),
        }
    }

    pub fn we_workshop_dir(&self) -> PathBuf {
        expand_home(&self.wallpaper_engine.workshop_dir)
    }
}

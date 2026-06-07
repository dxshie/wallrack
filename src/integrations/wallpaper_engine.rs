use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::integrations::Integration;
use crate::paths::{Paths, atomic_write};

pub const NAME: &str = "we";

pub struct WallpaperEngineIntegration;

#[derive(Debug, Deserialize, Default)]
struct ProjectJson {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    contentrating: Option<String>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

impl Integration for WallpaperEngineIntegration {
    fn name(&self) -> &'static str { NAME }

    fn index(&self, paths: &Paths, config: &Config) -> Result<Index> {
        paths.ensure_integration(NAME)?;
        let workshop = config.we_workshop_dir();
        if !workshop.is_dir() {
            return Err(anyhow!(
                "WE workshop dir not found: {} (set wallpaper_engine.workshop_dir in config)",
                workshop.display()
            ));
        }

        // Each <workshop>/<id>/project.json is one entry.
        let project_dirs: Vec<PathBuf> = std::fs::read_dir(&workshop)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("project.json").is_file())
            .collect();

        let entries: Vec<Entry> = project_dirs
            .par_iter()
            .filter_map(|dir| build_we_entry(dir).ok())
            .collect();

        let index = Index { integration: NAME.to_string(), entries };
        crate::integrations::write_index(paths, &index)?;
        Ok(index)
    }

    fn apply(&self, entry: &Entry, monitor: &str, paths: &Paths) -> Result<()> {
        if monitor.is_empty() {
            return Err(anyhow!("apply: no monitor given"));
        }
        let folder = &entry.source;
        let workshop_id = entry
            .workshop_id
            .clone()
            .or_else(|| folder.file_name().map(|s| s.to_string_lossy().to_string()))
            .ok_or_else(|| anyhow!("WE entry missing workshop id"))?;

        // Kill any running linux-wallpaperengine, then relaunch.
        let _ = Command::new("pkill").arg("-f").arg("linux-wallpaperengine").status();
        wait_we_gone();

        // setsid + uwsm to detach. Use `nohup` semantics via setsid.
        let mut cmd = Command::new("setsid");
        cmd.arg("uwsm")
            .arg("app")
            .arg("--")
            .arg("linux-wallpaperengine")
            .arg("--screenshot-delay").arg("1000")
            .arg("--disable-web-security")
            .arg("--autoplay-policy=no-user-gesture-required")
            .arg("--no-audio-processing")
            .arg("--disable-parallax")
            .arg("--silent")
            .arg("--no-fullscreen-pause")
            .arg("--scaling").arg("fill")
            .arg("--screen-root").arg(monitor)
            .arg("--bg").arg(&workshop_id);

        // Detach: ignore the child handle, redirect stdio.
        use std::process::Stdio;
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn().context("spawn linux-wallpaperengine")?;

        update_monitor_state(paths, monitor, &workshop_id)?;
        Ok(())
    }

    fn watch_dirs(&self, config: &Config) -> Vec<PathBuf> {
        let d = config.we_workshop_dir();
        if d.is_dir() { vec![d] } else { vec![] }
    }
}

fn build_we_entry(dir: &Path) -> Result<Entry> {
    let project_json = dir.join("project.json");
    let workshop_id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("bad WE dir name"))?
        .to_string();
    let body = std::fs::read_to_string(&project_json)
        .with_context(|| format!("read {}", project_json.display()))?;
    let meta: ProjectJson = serde_json::from_str(&body)
        .with_context(|| format!("parse {}", project_json.display()))?;

    let title = meta.title.filter(|s| !s.is_empty()).unwrap_or_else(|| workshop_id.clone());
    let rating = meta.contentrating.unwrap_or_default();
    let tags = meta.tags.unwrap_or_default();
    let preview = meta.preview.unwrap_or_default();
    let thumb = if preview.is_empty() { PathBuf::new() } else { dir.join(&preview) };

    Ok(Entry {
        integration: NAME.to_string(),
        id: dir.to_string_lossy().to_string(),
        title,
        source: dir.to_path_buf(),
        thumb,
        rating,
        tags,
        workshop_id: Some(workshop_id),
        subfolder: String::new(),
    })
}

fn wait_we_gone() {
    for _ in 0..50 {
        let still_running = Command::new("pgrep")
            .arg("-f")
            .arg("linux-wallpaperengine")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !still_running {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WeMonitorState {
    #[serde(flatten)]
    map: HashMap<String, String>,
}

fn update_monitor_state(paths: &Paths, monitor: &str, workshop_id: &str) -> Result<()> {
    let file = paths.we_monitor_state_file();
    let mut state: WeMonitorState = if file.exists() {
        let raw = std::fs::read_to_string(&file).unwrap_or_default();
        serde_json::from_str(&raw).unwrap_or_default()
    } else {
        WeMonitorState::default()
    };
    state.map.insert(monitor.to_string(), workshop_id.to_string());
    let body = serde_json::to_vec_pretty(&state)?;
    atomic_write(&file, &body)
}

pub fn read_monitor_state(paths: &Paths) -> HashMap<String, String> {
    let file = paths.we_monitor_state_file();
    if !file.exists() {
        return HashMap::new();
    }
    let raw = std::fs::read_to_string(&file).unwrap_or_default();
    serde_json::from_str::<WeMonitorState>(&raw)
        .map(|s| s.map)
        .unwrap_or_default()
}

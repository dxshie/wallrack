//! Wallpaper Engine integration — applies live wallpapers via
//! `linux-wallpaperengine`. Indexes each `<workshop>/<id>/project.json` as a
//! single entry, applied as a whole (no per-image drilling).

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
use crate::integrations::backend;
use crate::integrations::scan::ProjectJson;
use crate::integrations::thumb_filename_for;
use crate::paths::{Paths, atomic_write};
use crate::thumbnail;

pub const NAME: &str = "we";

pub struct WallpaperEngineIntegration;

impl Integration for WallpaperEngineIntegration {
    fn name(&self) -> &'static str {
        NAME
    }
    fn label(&self) -> &'static str {
        "WE"
    }

    // linux-wallpaperengine applies the whole project, so there is no
    // meaningful "drill into a subfolder" operation.
    fn supports_drill(&self) -> bool {
        false
    }

    fn index(&self, paths: &Paths, config: &Config) -> Result<Index> {
        paths.ensure_integration(NAME)?;
        let workshop = config.we_workshop_dir();
        if !workshop.is_dir() {
            return Err(anyhow!(
                "WE workshop dir not found: {} (set wallpaper_engine.workshop_dir in config)",
                workshop.display()
            ));
        }
        let thumbs_dir = paths.thumbs_dir(NAME);
        let thumb_size = config.thumbnails.size;

        // Each <workshop>/<id>/project.json is one entry.
        let project_dirs: Vec<PathBuf> = std::fs::read_dir(&workshop)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("project.json").is_file())
            .collect();

        let entries: Vec<Entry> = project_dirs
            .par_iter()
            .filter_map(|dir| build_we_entry(dir, &thumbs_dir, thumb_size).ok())
            .collect();

        let index = Index {
            integration: NAME.to_string(),
            entries,
        };
        crate::integrations::write_index(paths, &index)?;

        let notif_id = std::env::var("WALLRACK_NOTIF_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let mut cmd = std::process::Command::new("notify-send");
        cmd.arg("--expire-time=3000").arg("Wallrack").arg(format!(
            "we index built — {} wallpapers",
            index.entries.len()
        ));
        if let Some(id) = notif_id {
            cmd.arg(format!("--replace-id={id}"));
        }
        let _ = cmd.output();

        Ok(index)
    }

    fn apply(&self, entry: &Entry, monitor: &str, paths: &Paths, config: &Config) -> Result<()> {
        if monitor.is_empty() {
            return Err(anyhow!("apply: no monitor given"));
        }
        let Entry::Project {
            folder,
            workshop_id,
            ..
        } = entry
        else {
            return Err(anyhow!(
                "we apply called with a non-project entry: {}",
                entry.id()
            ));
        };

        // Replace any running linux-wallpaperengine before relaunching.
        let _ = Command::new("pkill")
            .arg("-f")
            .arg("linux-wallpaperengine")
            .status();
        wait_we_gone();

        let folder_str = folder.to_string_lossy();
        backend::run_apply_detached(
            &self.merged_backend(config),
            &[
                ("monitor", monitor),
                ("folder", folder_str.as_ref()),
                ("workshop_id", workshop_id.as_str()),
            ],
        )?;

        update_monitor_state(paths, monitor, workshop_id)?;
        Ok(())
    }

    fn watch_dirs(&self, config: &Config) -> Vec<PathBuf> {
        let d = config.we_workshop_dir();
        if d.is_dir() { vec![d] } else { vec![] }
    }

    fn backend<'a>(&self, config: &'a Config) -> &'a crate::config::BackendConfig {
        &config.wallpaper_engine.backend
    }

    fn default_backend(&self) -> crate::config::BackendConfig {
        crate::config::BackendConfig {
            // The default apply matches the previous hardcoded
            // linux-wallpaperengine invocation, minus the leading `setsid`
            // since `run_apply_detached` already wraps in one. Users on
            // non-uwsm setups can drop `uwsm app --` via config.
            apply_cmd: Some(
                r#"uwsm app -- linux-wallpaperengine --screenshot-delay 1000 --disable-web-security --autoplay-policy=no-user-gesture-required --no-audio-processing --disable-parallax --silent --no-fullscreen-pause --scaling fill --screen-root "{{monitor}}" --bg "{{workshop_id}}""#.into(),
            ),
            monitors_cmd: Some(r#"hyprctl monitors | awk '/^Monitor / {print $2}'"#.into()),
            // WE tracks its own per-monitor state — current_image_cmd is
            // unused for this integration.
            current_image_cmd: None,
        }
    }
}

fn build_we_entry(dir: &Path, thumbs_dir: &Path, thumb_size: u32) -> Result<Entry> {
    let project_json = dir.join("project.json");
    let workshop_id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("bad WE dir name"))?
        .to_string();
    let body = std::fs::read_to_string(&project_json)
        .with_context(|| format!("read {}", project_json.display()))?;
    let meta: ProjectJson =
        serde_json::from_str(&body).with_context(|| format!("parse {}", project_json.display()))?;

    let title = meta
        .title
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| workshop_id.clone());
    let rating = meta.contentrating.unwrap_or_default();
    let tags = meta.tags.unwrap_or_default();
    let preview = meta.preview.unwrap_or_default();
    // Workshop previews are typically JPG or animated GIF — neither renders
    // in fuzzel's libpng-only icon decoder. Run them through thumbnail::generate
    // so the cached thumb is a static PNG every picker can display. If
    // thumbnail generation fails (corrupt preview, unsupported codec) we fall
    // back to an empty thumb path; pickers skip the icon column gracefully.
    let thumb = if preview.is_empty() {
        PathBuf::new()
    } else {
        let preview_path = dir.join(&preview);
        let dst = thumbs_dir.join(thumb_filename_for(&preview_path));
        if thumbnail::generate(&preview_path, &dst, thumb_size).is_ok() {
            dst
        } else {
            PathBuf::new()
        }
    };

    Ok(Entry::Project {
        id: dir.to_string_lossy().to_string(),
        title,
        folder: dir.to_path_buf(),
        thumb,
        tags,
        rating,
        workshop_id,
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
    state
        .map
        .insert(monitor.to_string(), workshop_id.to_string());
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

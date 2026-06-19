//! Wallpaper Engine integration — applies live wallpapers via
//! `linux-wallpaperengine`. Indexes each `<workshop>/<id>/project.json` as a
//! single entry, applied as a whole (no per-image drilling).
//!
//! linux-wallpaperengine is a single-process beast: every monitor that wants
//! a live wallpaper must be passed as another `--screen-root M --bg ID` pair
//! on the same invocation. We can't "add a monitor" to a running process, so
//! every apply rebuilds the process from the union of WE-owned monitors held
//! in [`crate::applied`]. The pre-0.3 code only supported a single monitor at
//! a time because of that constraint; now [`Applied`] is the source of truth
//! and the command line is composed dynamically.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rayon::prelude::*;

use crate::applied::Applied;
use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::integrations::Integration;
use crate::integrations::backend;
use crate::integrations::scan::ProjectJson;
use crate::integrations::thumb_filename_for;
use crate::paths::Paths;
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
        let icon = crate::paths::icon_path();
        let mut cmd = std::process::Command::new("notify-send");
        cmd.arg("--expire-time=3000")
            .arg("-i")
            .arg(&icon)
            .arg("Wallrack")
            .arg(format!(
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
        let Entry::Project { workshop_id, .. } = entry else {
            return Err(anyhow!(
                "we apply called with a non-project entry: {}",
                entry.id()
            ));
        };

        // Stake out this monitor in the applied tree, then rebuild the WE
        // process from every WE-owned monitor — including any siblings the
        // user already had running with their own workshop wallpapers.
        let applied = Applied::open(paths.store())?;
        applied.set(monitor, NAME, workshop_id)?;
        let monitors = applied.by_integration(NAME);
        launch_for(&monitors, config)
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
            // Multi-monitor lives in {{we_screens}} — it expands to one
            // `--screen-root M --bg ID` pair per WE-owned monitor. The
            // single-monitor {{monitor}}/{{workshop_id}} placeholders are still
            // passed (filled with the just-applied entry) so pre-0.3 user
            // overrides keep producing a working single-screen command, but new
            // setups should template against {{we_screens}}.
            apply_cmd: Some(
                r#"uwsm app -- linux-wallpaperengine --screenshot-delay 1000 --disable-web-security --autoplay-policy=no-user-gesture-required --no-audio-processing --disable-parallax --silent --no-fullscreen-pause --scaling fill {{we_screens}}"#.into(),
            ),
            monitors_cmd: Some(r#"hyprctl monitors | awk '/^Monitor / {print $2}'"#.into()),
            // WE tracks its own per-monitor state via `applied` —
            // current_image_cmd is unused for this integration.
            current_image_cmd: None,
        }
    }
}

/// Kill any running `linux-wallpaperengine`, wait for it to die, then —
/// unless `monitors` is empty — spawn a new one with `--screen-root M --bg ID`
/// for every (monitor, workshop_id) pair. Centralizes the kill/spawn dance
/// so `apply`, `release_monitor`, and `wallrack applied restore` all go
/// through the same path.
pub fn launch_for(monitors: &BTreeMap<String, String>, config: &Config) -> Result<()> {
    // pkill -f matches the full command line — coarse but matches the
    // pre-0.3 behavior. Narrowing to a tracked pidfile would be nicer but
    // would silently fall through to no-op on stale pidfiles, leaving the
    // overlay alive. Leaving the broad kill until pidfile tracking is in.
    let _ = Command::new("pkill")
        .arg("-f")
        .arg("linux-wallpaperengine")
        .status();
    wait_we_gone();
    if monitors.is_empty() {
        return Ok(());
    }

    let screens = compose_screens(monitors);
    // For pre-0.3 single-monitor templates (using {{monitor}}/{{workshop_id}}),
    // pick a deterministic "primary" so the substitution still produces a
    // working command. BTreeMap orders by monitor name; first entry wins.
    let (primary_monitor, primary_workshop) = monitors
        .iter()
        .next()
        .map(|(m, w)| (m.as_str(), w.as_str()))
        .unwrap_or(("", ""));
    let integ = WallpaperEngineIntegration;
    backend::run_apply_detached(
        &integ.merged_backend(config),
        &[
            ("we_screens", screens.as_str()),
            ("monitor", primary_monitor),
            ("workshop_id", primary_workshop),
            ("folder", ""),
        ],
    )?;
    Ok(())
}

/// Drop `monitor` from the running WE process (if it was WE-owned), updating
/// `applied` and relaunching with the remaining WE monitors. No-op if the
/// monitor wasn't owned by WE. Called by image integrations' apply paths
/// before they set their own wallpaper so the WE overlay disappears.
pub fn release_monitor(monitor: &str, paths: &Paths, config: &Config) -> Result<()> {
    let applied = Applied::open(paths.store())?;
    let was_we = applied
        .get(monitor)
        .map(|e| e.integration == NAME)
        .unwrap_or(false);
    if !was_we {
        return Ok(());
    }
    applied.remove(monitor)?;
    let remaining = applied.by_integration(NAME);
    launch_for(&remaining, config)
}

fn compose_screens(monitors: &BTreeMap<String, String>) -> String {
    // Quote monitor names + workshop ids so unusual characters don't break
    // the shell-c'd command. Monitor names like `HDMI-A-1` and numeric
    // workshop ids are simple, but quoting is free defense.
    monitors
        .iter()
        .map(|(m, w)| format!("--screen-root {} --bg {}", shell_quote(m), shell_quote(w)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(s: &str) -> String {
    // Single-quoted POSIX string: replace any embedded `'` with `'\''`.
    format!("'{}'", s.replace('\'', "'\\''"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_screens_emits_one_pair_per_monitor() {
        let mut m = BTreeMap::new();
        m.insert("DP-1".to_string(), "1111".to_string());
        m.insert("DP-2".to_string(), "2222".to_string());
        let out = compose_screens(&m);
        // BTreeMap orders by key — DP-1 before DP-2.
        assert_eq!(
            out,
            "--screen-root 'DP-1' --bg '1111' --screen-root 'DP-2' --bg '2222'"
        );
    }

    #[test]
    fn compose_screens_handles_single_quote_in_values() {
        let mut m = BTreeMap::new();
        m.insert("DP-1".to_string(), "weird'id".to_string());
        let out = compose_screens(&m);
        assert_eq!(out, "--screen-root 'DP-1' --bg 'weird'\\''id'");
    }

    #[test]
    fn compose_screens_empty_map_is_empty_string() {
        let m: BTreeMap<String, String> = BTreeMap::new();
        assert_eq!(compose_screens(&m), "");
    }
}

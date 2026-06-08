use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::integrations::{Integration, thumb_filename_for};
use crate::paths::Paths;
use crate::thumbnail;

pub const NAME: &str = "wallpaper";

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp", "gif", "webp"];

pub struct WallpaperIntegration;

#[derive(Debug, Deserialize, Default)]
struct ProjectJson {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    contentrating: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

impl Integration for WallpaperIntegration {
    fn name(&self) -> &'static str { NAME }

    fn index(&self, paths: &Paths, config: &Config) -> Result<Index> {
        paths.ensure_integration(NAME)?;
        let thumbs_dir = paths.thumbs_dir(NAME);
        let thumb_size = config.thumbnails.size;

        let mut sources: Vec<EntrySource> = Vec::new();

        // 1. Steam workshop directory — directories that contain a project.json
        //    and image assets are treated as a "project root" with optional
        //    subfolders. This mirrors swww_we_picker_refresh.sh.
        if let Some(workshop) = config.wallpaper_steam_dir() {
            if workshop.is_dir() {
                eprintln!("wallrack: wallpaper: scanning workshop {}", workshop.display());
                collect_workshop_images(&workshop, &mut sources)?;
            } else {
                eprintln!("wallrack: wallpaper: workshop dir not found, skipping: {}", workshop.display());
            }
        }

        // 2. Plain wallpaper directories — recursive, no project.json.
        for dir in config.wallpaper_dirs() {
            if !dir.is_dir() {
                eprintln!("wallrack: wallpaper dir not found, skipping: {}", dir.display());
                continue;
            }
            eprintln!("wallrack: wallpaper: scanning plain dir {}", dir.display());
            collect_plain_images(&dir, &mut sources);
        }

        let total = sources.len();
        eprintln!("wallrack: wallpaper: {total} images found, generating thumbnails...");

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

    fn apply(&self, entry: &Entry, monitor: &str, _paths: &Paths) -> Result<()> {
        if monitor.is_empty() {
            return Err(anyhow!("apply: no monitor given"));
        }
        let img = &entry.source;
        if !img.exists() {
            return Err(anyhow!("image does not exist: {}", img.display()));
        }
        let status = Command::new("awww")
            .arg("img")
            .arg(img)
            .arg("--transition-type")
            .arg("center")
            .arg("-o")
            .arg(monitor)
            .status()
            .context("spawn awww")?;
        if !status.success() {
            return Err(anyhow!("awww exited with {status}"));
        }
        Ok(())
    }

    fn watch_dirs(&self, config: &Config) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(d) = config.wallpaper_steam_dir() {
            if d.is_dir() { dirs.push(d); }
        }
        for d in config.wallpaper_dirs() {
            if d.is_dir() { dirs.push(d); }
        }
        dirs
    }
}

#[derive(Debug)]
struct EntrySource {
    image: PathBuf,
    title: String,
    rating: String,
    tags: Vec<String>,
    workshop_id: Option<String>,
    project_root: Option<PathBuf>,
}

fn collect_workshop_images(workshop: &Path, out: &mut Vec<EntrySource>) -> Result<()> {
    // Workshop layout: <workshop>/<id>/project.json + image assets (possibly nested).
    for entry in std::fs::read_dir(workshop)? {
        let entry = entry?;
        let project_dir = entry.path();
        if !project_dir.is_dir() {
            continue;
        }
        let project_json = project_dir.join("project.json");
        if !project_json.is_file() {
            continue;
        }
        let workshop_id = project_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let (title, rating, tags) = match read_project_json(&project_json) {
            Ok(meta) => (
                meta.title.filter(|s| !s.is_empty()).unwrap_or_else(|| workshop_id.clone()),
                meta.contentrating.unwrap_or_default(),
                meta.tags.unwrap_or_default(),
            ),
            Err(_) => (workshop_id.clone(), String::new(), Vec::new()),
        };
        for img in walk_images(&project_dir, true) {
            out.push(EntrySource {
                image: img,
                title: title.clone(),
                rating: rating.clone(),
                tags: tags.clone(),
                workshop_id: Some(workshop_id.clone()),
                project_root: Some(project_dir.clone()),
            });
        }
    }
    Ok(())
}

fn collect_plain_images(dir: &Path, out: &mut Vec<EntrySource>) {
    for img in walk_images(dir, false) {
        let title = img
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_string();
        out.push(EntrySource {
            image: img,
            title,
            rating: String::new(),
            tags: Vec::new(),
            workshop_id: None,
            project_root: None,
        });
    }
}

fn walk_images(root: &Path, skip_preview: bool) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            if skip_preview
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase().contains("preview"))
                    .unwrap_or(false)
            {
                return false;
            }
            p.extension()
                .and_then(|s| s.to_str())
                .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect()
}

fn read_project_json(path: &Path) -> Result<ProjectJson> {
    let body = std::fs::read_to_string(path)?;
    let parsed: ProjectJson = serde_json::from_str(&body)?;
    Ok(parsed)
}

// One rayon-friendly progress reporter. Each worker calls `tick()`; we throttle
// rendering to ~16fps so the terminal isn't slammed with writes, and only one
// thread holds the render mutex at a time. Uses `\r` + `\x1b[K` (clear-to-EOL)
// in a single write so the line is updated atomically without flicker. When
// stderr is not a TTY (e.g. rofi script mode), sends notify-send notifications
// with replace-id so a single notification live-updates instead of spamming.
struct Progress {
    label: &'static str,
    total: usize,
    done: AtomicUsize,
    last_frame: Mutex<Instant>,
    rendered: AtomicBool,
    tty: bool,
    start: Instant,
    notif_id: Mutex<Option<String>>,
    last_notif: Mutex<Instant>,
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const BAR_WIDTH: usize = 30;
const FRAME_MS: u128 = 60;
const NOTIFY_THROTTLE_MS: u128 = 1_000;

impl Progress {
    fn new(label: &'static str, total: usize) -> Self {
        let initial_notif_id = std::env::var("WALLRACK_NOTIF_ID").ok()
            .filter(|s| !s.is_empty());
        Self {
            label,
            total,
            done: AtomicUsize::new(0),
            last_frame: Mutex::new(Instant::now()),
            rendered: AtomicBool::new(false),
            tty: std::io::stderr().is_terminal(),
            start: Instant::now(),
            notif_id: Mutex::new(initial_notif_id),
            last_notif: Mutex::new(Instant::now()),
        }
    }

    fn tick(&self) {
        let n = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if !self.tty {
            // try_lock: contending workers skip their tick rather than block.
            let Ok(mut last_notif) = self.last_notif.try_lock() else { return };
            let need_first = !self.rendered.load(Ordering::Relaxed);
            if !need_first && last_notif.elapsed().as_millis() < NOTIFY_THROTTLE_MS {
                return;
            }
            *last_notif = Instant::now();
            drop(last_notif);
            self.rendered.store(true, Ordering::Relaxed);
            self.notify_progress(n, false);
            return;
        }
        // try_lock so contending workers drop their tick instead of blocking the
        // par_iter; the next worker past the throttle window will pick it up.
        let Ok(mut last) = self.last_frame.try_lock() else { return };
        let need_first = !self.rendered.load(Ordering::Relaxed);
        if !need_first && last.elapsed().as_millis() < FRAME_MS {
            return;
        }
        *last = Instant::now();
        drop(last);
        self.rendered.store(true, Ordering::Relaxed);
        self.render(n);
    }

    fn notify_progress(&self, n: usize, done: bool) {
        let pct = if self.total == 0 { 100 } else { (n * 100 / self.total).min(100) };
        let body = if done {
            format!("{} index built — {} wallpapers", self.label, n)
        } else {
            format!("Indexing {} — {}/{} ({}%)", self.label, n, self.total, pct)
        };
        let expire_ms = if done { "3000" } else { "0" };

        let mut notif_id = self.notif_id.lock().unwrap();
        let mut cmd = Command::new("notify-send");
        cmd.arg("--print-id")
           .arg(format!("--expire-time={expire_ms}"))
           .arg("Wallrack")
           .arg(&body);
        if let Some(ref id) = *notif_id {
            cmd.arg(format!("--replace-id={id}"));
        }
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !id_str.is_empty() {
                    *notif_id = Some(id_str);
                }
            }
        }
    }

    fn render(&self, n: usize) {
        let frac = if self.total == 0 {
            1.0
        } else {
            (n as f32 / self.total as f32).min(1.0)
        };
        let filled = (frac * BAR_WIDTH as f32).round() as usize;
        let pct = (frac * 100.0) as u32;
        let spin = SPINNER[(self.start.elapsed().as_millis() / 80) as usize % SPINNER.len()];
        let mut bar = String::with_capacity(BAR_WIDTH * 3);
        for i in 0..BAR_WIDTH {
            bar.push(if i < filled { '█' } else { '░' });
        }
        let line = format!(
            "\rwallrack: {} {} [{}] {}/{} ({}%)\x1b[K",
            self.label, spin, bar, n, self.total, pct
        );
        let stderr = std::io::stderr();
        let mut handle = stderr.lock();
        let _ = handle.write_all(line.as_bytes());
        let _ = handle.flush();
    }

    fn finish(&self) {
        let n = self.done.load(Ordering::Relaxed);
        if self.tty {
            // Force a final render in case the last tick was throttled out.
            self.render(n);
            let _ = writeln!(std::io::stderr());
        } else {
            self.notify_progress(n, true);
        }
    }
}

fn build_entry(src: &EntrySource, thumbs_dir: &Path, size: u32) -> Result<Entry> {
    let thumb_name = thumb_filename_for(&src.image);
    let thumb = thumbs_dir.join(&thumb_name);
    let _ = thumbnail::generate(&src.image, &thumb, size);

    let subfolder = match &src.project_root {
        Some(root) => src.image
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        None => String::new(),
    };

    Ok(Entry {
        integration: NAME.to_string(),
        id: src.image.to_string_lossy().to_string(),
        title: src.title.clone(),
        source: src.image.clone(),
        thumb,
        rating: src.rating.clone(),
        tags: src.tags.clone(),
        workshop_id: src.workshop_id.clone(),
        subfolder,
    })
}

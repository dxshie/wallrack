use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

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

        // Throttled progress: each rayon worker bumps a counter; we print
        // every 50 items so output stays useful even on huge libraries.
        let done = AtomicUsize::new(0);
        let entries: Vec<Entry> = sources
            .par_iter()
            .filter_map(|src| {
                let entry = build_entry(src, &thumbs_dir, thumb_size).ok();
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 50 == 0 || n == total {
                    eprintln!("wallrack: wallpaper: {n}/{total} processed");
                }
                entry
            })
            .collect();

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

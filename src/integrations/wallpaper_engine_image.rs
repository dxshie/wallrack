//! Wallpaper Engine *Image* integration — extracts still images from Steam
//! Workshop wallpaper-engine projects and applies them like any normal image
//! wallpaper. Use this for the "WE pack but I just want the pictures" flow;
//! the live linux-wallpaperengine integration lives in `wallpaper_engine.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::config::Config;
use crate::entry::{Entry, Index};
use crate::integrations::backend;
use crate::integrations::progress::Progress;
use crate::integrations::{IMAGE_EXTS, Integration, thumb_filename_for};
use crate::paths::Paths;
use crate::thumbnail;

pub const NAME: &str = "we_image";

pub struct WallpaperEngineImageIntegration;

#[derive(Debug, Deserialize, Default)]
struct ProjectJson {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    contentrating: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

impl Integration for WallpaperEngineImageIntegration {
    fn name(&self) -> &'static str { NAME }
    fn label(&self) -> &'static str { "WE Image" }

    fn index(&self, paths: &Paths, config: &Config) -> Result<Index> {
        paths.ensure_integration(NAME)?;
        let thumbs_dir = paths.thumbs_dir(NAME);
        let thumb_size = config.thumbnails.size;

        let workshop = config.we_image_workshop_dir();
        if !workshop.is_dir() {
            eprintln!(
                "wallrack: we_image: workshop dir not found: {}",
                workshop.display()
            );
            let index = Index { integration: NAME.to_string(), entries: Vec::new() };
            crate::integrations::write_index(paths, &index)?;
            return Ok(index);
        }

        eprintln!("wallrack: we_image: scanning workshop {}", workshop.display());
        let mut sources: Vec<EntrySource> = Vec::new();
        collect_workshop_images(&workshop, &mut sources)?;

        let total = sources.len();
        eprintln!("wallrack: we_image: {total} images found, generating thumbnails...");

        let progress = Progress::new("we_image", total);
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
        let d = config.we_image_workshop_dir();
        if d.is_dir() { vec![d] } else { vec![] }
    }

    fn backend<'a>(&self, config: &'a Config) -> &'a crate::config::BackendConfig {
        &config.wallpaper_engine_image.backend
    }

    fn default_backend(&self) -> crate::config::BackendConfig {
        crate::integrations::wallpaper::image_backend_defaults()
    }
}

#[derive(Debug)]
struct EntrySource {
    image: PathBuf,
    title: String,
    rating: String,
    tags: Vec<String>,
    workshop_id: String,
    project_root: PathBuf,
}

fn collect_workshop_images(workshop: &Path, out: &mut Vec<EntrySource>) -> Result<()> {
    // Layout: <workshop>/<id>/project.json + image assets (possibly nested).
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
        for img in walk_images(&project_dir) {
            out.push(EntrySource {
                image: img,
                title: title.clone(),
                rating: rating.clone(),
                tags: tags.clone(),
                workshop_id: workshop_id.clone(),
                project_root: project_dir.clone(),
            });
        }
    }
    Ok(())
}

fn walk_images(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            // Skip the project's preview image — that's intended as a
            // thumbnail, not a wallpaper.
            if p.file_name()
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

    let subfolder = src.image
        .parent()
        .and_then(|p| p.strip_prefix(&src.project_root).ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(Entry {
        integration: NAME.to_string(),
        id: src.image.to_string_lossy().to_string(),
        title: src.title.clone(),
        source: src.image.clone(),
        thumb,
        rating: src.rating.clone(),
        tags: src.tags.clone(),
        workshop_id: Some(src.workshop_id.clone()),
        subfolder,
    })
}

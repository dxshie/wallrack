//! Shared filesystem-scan helpers used by the image-based integrations
//! (`wallpaper`, `we_image`) and by the live WE integration for its
//! `project.json` metadata.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use walkdir::WalkDir;

use super::IMAGE_EXTS;

/// Recursively collect image files under `root`. When `skip_preview` is set,
/// filenames containing the substring "preview" (case-insensitive) are
/// excluded — this is how the `we_image` integration keeps the workshop
/// preview thumbnails out of the wallpaper rotation.
pub fn walk_images(root: &Path, skip_preview: bool) -> Vec<PathBuf> {
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

/// Subset of `project.json` fields wallrack reads. Both WE integrations
/// (`we` and `we_image`) share the same metadata; the `preview` field is
/// only used by the live `we` integration for its workshop thumbnail.
#[derive(Debug, Deserialize, Default)]
pub struct ProjectJson {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub contentrating: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

pub fn read_project_json(path: &Path) -> Result<ProjectJson> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let parsed: ProjectJson = serde_json::from_str(&body)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(parsed)
}

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A single indexed item — image wallpaper, WE project, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Which integration this entry belongs to.
    pub integration: String,
    /// Stable identifier used by favorites/state. For images this is the
    /// absolute path; for WE projects it is the project folder path.
    pub id: String,
    /// Human-readable title shown in the picker.
    pub title: String,
    /// Path to the source asset (image file or WE project folder).
    pub source: PathBuf,
    /// Path to the rendered thumbnail (may not exist on disk if generation failed).
    pub thumb: PathBuf,
    /// Content rating (e.g. "Everyone"). Empty when unknown.
    #[serde(default)]
    pub rating: String,
    /// Free-form tags, lowercase recommended.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Workshop/project ID when applicable.
    #[serde(default)]
    pub workshop_id: Option<String>,
    /// Subfolder relative to project root — empty when at the root.
    #[serde(default)]
    pub subfolder: String,
}

/// The cached index for a single integration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub integration: String,
    pub entries: Vec<Entry>,
}

//! Picker state — a small key/value store, persisted as JSON.
//!
//! On-disk the file is a flat `BTreeMap<String, String>` so the picker
//! shells can read/write keys via `wallrack state get/set/unset` without
//! schema awareness. The Rust callers go through typed accessors below —
//! `state.picker_mode()`, `state.view_mode()`, etc. — which centralizes
//! every "did I spell `tag_edit_target` right?" risk into this one module.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::atomic_write;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(flatten)]
    values: BTreeMap<String, String>,
}

impl State {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read state {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let parsed: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parse state {}", path.display()))?;
        Ok(parsed)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serialize state")?;
        atomic_write(path, &body)
    }

    // ─── raw string access (for `wallrack state get/set/unset`) ──────────

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let v = value.into();
        if v.is_empty() {
            self.values.remove(key);
        } else {
            self.values.insert(key.to_string(), v);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }

    pub fn all(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    // ─── typed accessors ─────────────────────────────────────────────────

    pub fn picker_mode(&self) -> PickerMode {
        PickerMode::parse(self.get(keys::PICKER_MODE).unwrap_or(""))
    }

    pub fn view_mode(&self) -> ViewMode {
        ViewMode::parse(self.get(keys::VIEW_MODE).unwrap_or(""))
    }

    pub fn rating_filter(&self) -> RatingFilter {
        RatingFilter::parse(self.get(keys::RATING).unwrap_or(""))
    }

    pub fn drill_path(&self) -> &str {
        self.get(keys::DRILL_PATH).unwrap_or("")
    }

    pub fn tag_filter(&self) -> &str {
        self.get(keys::TAG_FILTER).unwrap_or("")
    }

    pub fn tag_mode_selecting(&self) -> bool {
        self.get(keys::TAG_MODE) == Some("selecting")
    }

    pub fn tag_edit_target(&self) -> &str {
        self.get(keys::TAG_EDIT_TARGET).unwrap_or("")
    }

    pub fn tag_add_mode_on(&self) -> bool {
        self.get(keys::TAG_ADD_MODE) == Some("on")
    }

    pub fn booru_site(&self) -> Option<&str> {
        let s = self.get(keys::BOORU_SITE)?;
        (!s.is_empty()).then_some(s)
    }

    pub fn booru_query(&self) -> &str {
        self.get(keys::BOORU_QUERY).unwrap_or("")
    }

    pub fn booru_page(&self) -> u32 {
        self.get(keys::BOORU_PAGE)
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
    }

    pub fn booru_search_mode_on(&self) -> bool {
        self.get(keys::BOORU_SEARCH_MODE) == Some("on")
    }

    /// Compute the current picker view. The sub-view checks run in priority
    /// order (add wins over edit wins over tag-select), so a typo in any one
    /// branch can't accidentally route through the wrong sub-view.
    pub fn picker_view(&self) -> PickerView {
        if self.tag_add_mode_on() {
            return PickerView::TagAdd;
        }
        if !self.tag_edit_target().is_empty() {
            return PickerView::TagEditor;
        }
        if self.tag_mode_selecting() {
            return PickerView::TagSelect;
        }
        if self.picker_mode() == PickerMode::Booru {
            return PickerView::Booru;
        }
        PickerView::Default
    }
}

// ─── typed picker-mode values ─────────────────────────────────────────

/// Active source (matches integration `name()`). The serialized form is
/// the integration key — same as what the picker shell writes via
/// `wallrack state set picker_mode <key>`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    #[default]
    Wallpaper,
    WeImage,
    We,
    Booru,
}

impl PickerMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "we_image" | "wallpaper_engine_image" => Self::WeImage,
            "we" | "wallpaper_engine" => Self::We,
            "booru" => Self::Booru,
            _ => Self::Wallpaper,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wallpaper => "wallpaper",
            Self::WeImage => "we_image",
            Self::We => "we",
            Self::Booru => "booru",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    All,
    Favorites,
}

impl ViewMode {
    pub fn parse(s: &str) -> Self {
        if s == "favorites" { Self::Favorites } else { Self::All }
    }
    pub fn favorites_only(self) -> bool {
        matches!(self, Self::Favorites)
    }
}

/// Rating filter cycled by the picker. `All` is the no-filter sentinel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RatingFilter {
    #[default]
    All,
    Mature,
    Questionable,
    Everyone,
}

impl RatingFilter {
    pub fn parse(s: &str) -> Self {
        match s {
            "Mature" => Self::Mature,
            "Questionable" => Self::Questionable,
            "Everyone" => Self::Everyone,
            _ => Self::All,
        }
    }
    /// String to match against `Entry::rating`; `None` for the no-filter
    /// case so callers can short-circuit.
    pub fn as_filter(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Mature => Some("Mature"),
            Self::Questionable => Some("Questionable"),
            Self::Everyone => Some("Everyone"),
        }
    }
}

/// The picker's current view, derived from state. Replaces the chain of
/// `if tag_add_mode == "on" { ... } else if !tag_edit_target.is_empty() { ... }`
/// branches the original `cmd_view` carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerView {
    /// Add-tag prompt (with optional free-form input).
    TagAdd,
    /// Per-entry tag editor.
    TagEditor,
    /// Tag-filter selection (the "pick a tag to filter by" view).
    TagSelect,
    /// Booru search results / search prompt.
    Booru,
    /// Default flat / grouped / drilled view of the current integration.
    Default,
}

// String constants for the picker keys. The shells use these via
// `wallrack state set/get`, so they're part of the public surface.
pub mod keys {
    pub const PICKER_MODE: &str = "picker_mode";
    pub const VIEW_MODE: &str = "view_mode";
    pub const DRILL_PATH: &str = "drill_path";
    pub const TAG_FILTER: &str = "tag_filter";
    pub const TAG_MODE: &str = "tag_mode";
    pub const TAG_EDIT_TARGET: &str = "tag_edit_target";
    pub const TAG_ADD_MODE: &str = "tag_add_mode";
    pub const RATING: &str = "rating";
    // booru-specific picker state.
    pub const BOORU_SITE: &str = "booru_site";
    pub const BOORU_QUERY: &str = "booru_query";
    pub const BOORU_PAGE: &str = "booru_page";
    pub const BOORU_SEARCH_MODE: &str = "booru_search_mode";
    // Cross-integration apply hand-off — booru downloads then routes through
    // the wallpaper monitor picker, but picker_mode stays `booru` so the user
    // lands back in search on the next open.
    pub const APPLY_INTEGRATION_OVERRIDE: &str = "apply_integration_override";
}

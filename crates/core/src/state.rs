//! Picker state — sled-backed key/value store.
//!
//! Wire format: each picker-state key is a sled key; the value is the raw
//! string. The picker shells (`wallrack state get/set/unset`) operate on
//! string keys directly; Rust callers go through the typed accessors
//! below — `state.picker_mode()`, `state.view_mode()`, etc. — so a typo
//! in the field name is a compile error rather than a silent miss.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use sled::{Db, Tree};

use crate::store::TREE_STATE;

/// Sentinel key written by the migration step. Hidden from `all()` and the
/// `state dump` CLI to avoid leaking implementation detail.
const MIGRATION_SENTINEL: &str = "__migrated_from_json";

pub struct State {
    tree: Tree,
}

impl State {
    pub fn open(db: &Db) -> Result<Self> {
        let tree = db
            .open_tree(TREE_STATE)
            .with_context(|| format!("open sled tree `{TREE_STATE}`"))?;
        Ok(Self { tree })
    }

    // ─── raw string access (for `wallrack state get/set/unset`) ──────────

    pub fn get(&self, key: &str) -> Option<String> {
        let bytes = self.tree.get(key).ok().flatten()?;
        std::str::from_utf8(&bytes).ok().map(|s| s.to_string())
    }

    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or_else(|| default.to_string())
    }

    pub fn set(&self, key: &str, value: impl Into<String>) -> Result<()> {
        let v = value.into();
        if v.is_empty() {
            self.tree.remove(key)?;
        } else {
            self.tree.insert(key, v.as_bytes())?;
        }
        self.tree.flush()?;
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        self.tree.remove(key)?;
        self.tree.flush()?;
        Ok(())
    }

    pub fn all(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for kv in self.tree.iter() {
            let Ok((k, v)) = kv else { continue };
            let Ok(key) = std::str::from_utf8(&k) else { continue };
            if key.starts_with("__") || key == MIGRATION_SENTINEL {
                continue;
            }
            let Ok(value) = std::str::from_utf8(&v) else { continue };
            out.insert(key.to_string(), value.to_string());
        }
        out
    }

    // ─── typed accessors ─────────────────────────────────────────────────

    pub fn picker_mode(&self) -> PickerMode {
        PickerMode::parse(self.get(keys::PICKER_MODE).as_deref().unwrap_or(""))
    }

    pub fn view_mode(&self) -> ViewMode {
        ViewMode::parse(self.get(keys::VIEW_MODE).as_deref().unwrap_or(""))
    }

    pub fn rating_filter(&self) -> RatingFilter {
        RatingFilter::parse(self.get(keys::RATING).as_deref().unwrap_or(""))
    }

    pub fn drill_path(&self) -> String {
        self.get(keys::DRILL_PATH).unwrap_or_default()
    }

    pub fn tag_filter(&self) -> String {
        self.get(keys::TAG_FILTER).unwrap_or_default()
    }

    pub fn tag_mode_selecting(&self) -> bool {
        self.get(keys::TAG_MODE).as_deref() == Some("selecting")
    }

    pub fn tag_edit_target(&self) -> String {
        self.get(keys::TAG_EDIT_TARGET).unwrap_or_default()
    }

    pub fn tag_add_mode_on(&self) -> bool {
        self.get(keys::TAG_ADD_MODE).as_deref() == Some("on")
    }

    pub fn booru_site(&self) -> Option<String> {
        let s = self.get(keys::BOORU_SITE)?;
        (!s.is_empty()).then_some(s)
    }

    pub fn booru_query(&self) -> String {
        self.get(keys::BOORU_QUERY).unwrap_or_default()
    }

    pub fn booru_page(&self) -> u32 {
        self.get(keys::BOORU_PAGE)
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
    }

    pub fn booru_search_mode_on(&self) -> bool {
        self.get(keys::BOORU_SEARCH_MODE).as_deref() == Some("on")
    }

    /// Compute the current picker view. The sub-view checks run in priority
    /// order (add wins over edit wins over tag-select).
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
    pub fn as_filter(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Mature => Some("Mature"),
            Self::Questionable => Some("Questionable"),
            Self::Everyone => Some("Everyone"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerView {
    TagAdd,
    TagEditor,
    TagSelect,
    Booru,
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
    pub const BOORU_SITE: &str = "booru_site";
    pub const BOORU_QUERY: &str = "booru_query";
    pub const BOORU_PAGE: &str = "booru_page";
    pub const BOORU_SEARCH_MODE: &str = "booru_search_mode";
    pub const APPLY_INTEGRATION_OVERRIDE: &str = "apply_integration_override";
}

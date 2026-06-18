use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::atomic_write;

/// Picker state — small key/value store, persisted as JSON.
///
/// Free-form so the shell side can stash any extra hints without a
/// schema migration.
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
}

// Typed accessors for the keys the picker uses. Keeping them in one place
// avoids the "did I spell view_mode right?" class of bug.
pub mod keys {
    pub const PICKER_MODE:     &str = "picker_mode";
    pub const VIEW_MODE:       &str = "view_mode";
    pub const DRILL_PATH:      &str = "drill_path";
    pub const TAG_FILTER:      &str = "tag_filter";
    pub const TAG_MODE:        &str = "tag_mode";
    pub const TAG_EDIT_TARGET: &str = "tag_edit_target";
    pub const TAG_ADD_MODE:    &str = "tag_add_mode";
    pub const RATING:          &str = "rating";
    // booru-specific picker state. The booru integration is search-driven —
    // these track the "what does my current page show?" context so the
    // frontend can paginate / re-site without re-typing the query.
    pub const BOORU_SITE:        &str = "booru_site";
    pub const BOORU_QUERY:       &str = "booru_query";
    pub const BOORU_PAGE:        &str = "booru_page";
    pub const BOORU_SEARCH_MODE: &str = "booru_search_mode";
    // Cross-integration apply hand-off. The booru flow downloads then routes
    // into the wallpaper monitor picker, but picker_mode stays `booru` so the
    // user lands back in the search on the next open — this key tells the
    // apply step "this round, use the wallpaper integration, not picker_mode".
    pub const APPLY_INTEGRATION_OVERRIDE: &str = "apply_integration_override";
}

//! User-applied tag overrides — additive and subtractive edits layered on
//! top of each integration's native tags (project.json for WE, none for
//! plain images, etc.).
//!
//! Storage: `~/.cache/wallrack/tags.json`, keyed by integration → entry id.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::entry::Index;
use crate::paths::atomic_write;

/// Per-integration "known tags" catalog. Populated by:
///   - indexing (union of native tags from project.json etc.)
///   - `wallrack tag add` / `tag set` (so a user-added tag is immediately
///     suggestable in the picker without waiting for a re-index)
///   - `wallrack tag create` (declare a tag without assigning it yet)
///
/// The catalog is the source of truth for "what tags can I apply" — distinct
/// from the per-entry overrides which track "which tags apply to this entry".
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TagCatalog {
    #[serde(flatten)]
    by_integration: BTreeMap<String, BTreeSet<String>>,
}

impl TagCatalog {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read tag catalog {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let parsed: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parse tag catalog {}", path.display()))?;
        Ok(parsed)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serialize tag catalog")?;
        atomic_write(path, &body)
    }

    pub fn list(&self, integration: &str) -> Vec<String> {
        self.by_integration
            .get(integration)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn contains(&self, integration: &str, tag: &str) -> bool {
        self.by_integration
            .get(integration)
            .map(|s| s.contains(tag))
            .unwrap_or(false)
    }

    /// Insert a single tag. Returns true if newly added.
    pub fn add(&mut self, integration: &str, tag: &str) -> bool {
        if tag.is_empty() {
            return false;
        }
        self.by_integration
            .entry(integration.to_string())
            .or_default()
            .insert(tag.to_string())
    }

    pub fn extend<I: IntoIterator<Item = String>>(&mut self, integration: &str, tags: I) {
        let set = self.by_integration.entry(integration.to_string()).or_default();
        for t in tags {
            if !t.is_empty() {
                set.insert(t);
            }
        }
    }

    pub fn remove(&mut self, integration: &str, tag: &str) -> bool {
        let removed = self
            .by_integration
            .get_mut(integration)
            .map(|s| s.remove(tag))
            .unwrap_or(false);
        if let Some(s) = self.by_integration.get(integration) {
            if s.is_empty() {
                self.by_integration.remove(integration);
            }
        }
        removed
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryOverride {
    #[serde(default)]
    pub added: BTreeSet<String>,
    #[serde(default)]
    pub removed: BTreeSet<String>,
}

impl EntryOverride {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TagOverrides {
    #[serde(flatten)]
    by_integration: BTreeMap<String, BTreeMap<String, EntryOverride>>,
}

impl TagOverrides {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read tags {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let parsed: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parse tags {}", path.display()))?;
        Ok(parsed)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serialize tag overrides")?;
        atomic_write(path, &body)
    }

    /// Add `tag` to the effective set. Cancels a prior `remove` of the same tag.
    pub fn add(&mut self, integration: &str, id: &str, tag: &str) {
        let entry = self.entry_mut(integration, id);
        entry.removed.remove(tag);
        entry.added.insert(tag.to_string());
    }

    /// Remove `tag` from the effective set. Cancels a prior `add` of the same
    /// tag and records a "hide native" marker so a tag inherited from
    /// project.json stays hidden.
    pub fn remove(&mut self, integration: &str, id: &str, tag: &str) {
        let entry = self.entry_mut(integration, id);
        entry.added.remove(tag);
        entry.removed.insert(tag.to_string());
        self.gc(integration, id);
    }

    /// Replace the effective tag set on this entry with `new_tags`. `native`
    /// is the entry's tags before overrides — used to compute the minimal
    /// added/removed deltas so the override survives index regenerations
    /// (added tags don't vanish if a project.json is re-read).
    pub fn set(&mut self, integration: &str, id: &str, new_tags: &[String], native: &[String]) {
        let new: BTreeSet<String> = new_tags.iter().cloned().collect();
        let native: BTreeSet<String> = native.iter().cloned().collect();
        let entry = self.entry_mut(integration, id);
        entry.added = new.difference(&native).cloned().collect();
        entry.removed = native.difference(&new).cloned().collect();
        self.gc(integration, id);
    }

    pub fn clear(&mut self, integration: &str, id: &str) {
        if let Some(by_id) = self.by_integration.get_mut(integration) {
            by_id.remove(id);
            if by_id.is_empty() {
                self.by_integration.remove(integration);
            }
        }
    }

    pub fn get(&self, integration: &str, id: &str) -> Option<&EntryOverride> {
        self.by_integration.get(integration)?.get(id)
    }

    /// Layer overrides over an integration's index in place.
    pub fn apply_to(&self, idx: &mut Index) {
        let Some(by_id) = self.by_integration.get(&idx.integration) else { return };
        for entry in &mut idx.entries {
            if let Some(ov) = by_id.get(&entry.id) {
                let mut effective: BTreeSet<String> = entry
                    .tags
                    .iter()
                    .filter(|t| !ov.removed.contains(*t))
                    .cloned()
                    .collect();
                effective.extend(ov.added.iter().cloned());
                entry.tags = effective.into_iter().collect();
            }
        }
    }

    fn entry_mut(&mut self, integration: &str, id: &str) -> &mut EntryOverride {
        self.by_integration
            .entry(integration.to_string())
            .or_default()
            .entry(id.to_string())
            .or_default()
    }

    fn gc(&mut self, integration: &str, id: &str) {
        let drop_entry = self
            .by_integration
            .get(integration)
            .and_then(|by_id| by_id.get(id))
            .map(|e| e.is_empty())
            .unwrap_or(false);
        if drop_entry {
            self.clear(integration, id);
        }
    }
}

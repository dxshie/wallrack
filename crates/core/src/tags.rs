//! User-applied tag overrides — additive and subtractive edits layered on
//! top of each integration's native tags. Sled-backed: the catalog is a
//! membership tree (composite key, empty value); the overrides tree maps
//! `<integration>\0<id>` to a JSON `EntryOverride` blob.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sled::{Db, Tree};

use crate::entry::Index;
use crate::store::{KEY_SEP, TREE_TAG_CATALOG, TREE_TAG_OVERRIDES, composite_key, read_json, write_json};

/// Per-integration "known tags" catalog. Populated by:
///   - indexing (union of native tags from project.json etc.)
///   - `wallrack tag add` / `tag set` (so a user-added tag is immediately
///     suggestable in the picker without waiting for a re-index)
///   - `wallrack tag create` (declare a tag without assigning it yet)
pub struct TagCatalog {
    tree: Tree,
}

impl TagCatalog {
    pub fn open(db: &Db) -> Result<Self> {
        let tree = db
            .open_tree(TREE_TAG_CATALOG)
            .with_context(|| format!("open sled tree `{TREE_TAG_CATALOG}`"))?;
        Ok(Self { tree })
    }

    pub fn list(&self, integration: &str) -> Vec<String> {
        let mut prefix = integration.as_bytes().to_vec();
        prefix.push(KEY_SEP);
        let mut out = Vec::new();
        for kv in self.tree.scan_prefix(&prefix) {
            let Ok((k, _)) = kv else { continue };
            if let Some(rest) = k.strip_prefix(prefix.as_slice()) {
                if let Ok(s) = std::str::from_utf8(rest) {
                    if !s.starts_with("__") {
                        out.push(s.to_string());
                    }
                }
            }
        }
        out
    }

    pub fn contains(&self, integration: &str, tag: &str) -> bool {
        self.tree
            .contains_key(composite_key(integration, tag))
            .unwrap_or(false)
    }

    /// Insert a single tag. Returns true if newly added.
    pub fn add(&self, integration: &str, tag: &str) -> bool {
        if tag.is_empty() {
            return false;
        }
        let key = composite_key(integration, tag);
        let inserted = self
            .tree
            .insert(&key, &[])
            .map(|prev| prev.is_none())
            .unwrap_or(false);
        let _ = self.tree.flush();
        inserted
    }

    pub fn extend<I: IntoIterator<Item = String>>(&self, integration: &str, tags: I) -> bool {
        let mut any_new = false;
        for t in tags {
            if t.is_empty() {
                continue;
            }
            if self.add(integration, &t) {
                any_new = true;
            }
        }
        any_new
    }

    pub fn remove(&self, integration: &str, tag: &str) -> bool {
        let key = composite_key(integration, tag);
        let removed = self
            .tree
            .remove(&key)
            .map(|prev| prev.is_some())
            .unwrap_or(false);
        let _ = self.tree.flush();
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

pub struct TagOverrides {
    tree: Tree,
}

impl TagOverrides {
    pub fn open(db: &Db) -> Result<Self> {
        let tree = db
            .open_tree(TREE_TAG_OVERRIDES)
            .with_context(|| format!("open sled tree `{TREE_TAG_OVERRIDES}`"))?;
        Ok(Self { tree })
    }

    /// Add `tag` to the effective set. Cancels a prior `remove` of the same tag.
    pub fn add(&self, integration: &str, id: &str, tag: &str) -> Result<()> {
        let mut entry = self.get(integration, id)?.unwrap_or_default();
        entry.removed.remove(tag);
        entry.added.insert(tag.to_string());
        self.store(integration, id, &entry)
    }

    /// Remove `tag` from the effective set. Cancels a prior `add` of the same
    /// tag and records a "hide native" marker so a tag inherited from
    /// project.json stays hidden.
    pub fn remove(&self, integration: &str, id: &str, tag: &str) -> Result<()> {
        let mut entry = self.get(integration, id)?.unwrap_or_default();
        entry.added.remove(tag);
        entry.removed.insert(tag.to_string());
        self.store(integration, id, &entry)
    }

    /// Replace the effective tag set on this entry with `new_tags`. `native`
    /// is the entry's tags before overrides — used to compute the minimal
    /// added/removed deltas so the override survives index regenerations.
    pub fn set(&self, integration: &str, id: &str, new_tags: &[String], native: &[String]) -> Result<()> {
        let new: BTreeSet<String> = new_tags.iter().cloned().collect();
        let native: BTreeSet<String> = native.iter().cloned().collect();
        let entry = EntryOverride {
            added: new.difference(&native).cloned().collect(),
            removed: native.difference(&new).cloned().collect(),
        };
        self.store(integration, id, &entry)
    }

    pub fn clear(&self, integration: &str, id: &str) -> Result<()> {
        let key = composite_key(integration, id);
        self.tree.remove(&key)?;
        self.tree.flush()?;
        Ok(())
    }

    pub fn get(&self, integration: &str, id: &str) -> Result<Option<EntryOverride>> {
        let key = composite_key(integration, id);
        read_json(&self.tree, &key)
    }

    /// Layer overrides over an integration's index in place.
    pub fn apply_to(&self, idx: &mut Index) {
        for entry in &mut idx.entries {
            let key = composite_key(&idx.integration, entry.id());
            let Ok(Some(ov)) = read_json::<EntryOverride>(&self.tree, &key) else {
                continue;
            };
            let mut effective: BTreeSet<String> = entry
                .tags()
                .iter()
                .filter(|t| !ov.removed.contains(*t))
                .cloned()
                .collect();
            effective.extend(ov.added.iter().cloned());
            entry.set_tags(effective.into_iter().collect());
        }
    }

    fn store(&self, integration: &str, id: &str, entry: &EntryOverride) -> Result<()> {
        let key = composite_key(integration, id);
        if entry.is_empty() {
            self.tree.remove(&key)?;
        } else {
            write_json(&self.tree, &key, entry)?;
        }
        self.tree.flush()?;
        Ok(())
    }
}

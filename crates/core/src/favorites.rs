//! Per-integration favorites — sled-backed. Each favorited entry is one
//! key/value pair in the `favorites` tree, keyed by `<integration>\0<id>`
//! with an empty marker value. Per-integration listings use a prefix scan.

use anyhow::{Context, Result};
use sled::{Db, Tree};

use crate::store::{KEY_SEP, TREE_FAVORITES, composite_key};

pub struct Favorites {
    tree: Tree,
}

impl Favorites {
    pub fn open(db: &Db) -> Result<Self> {
        let tree = db
            .open_tree(TREE_FAVORITES)
            .with_context(|| format!("open sled tree `{TREE_FAVORITES}`"))?;
        Ok(Self { tree })
    }

    pub fn is_favorite(&self, integration: &str, id: &str) -> bool {
        let key = composite_key(integration, id);
        self.tree.contains_key(&key).unwrap_or(false)
    }

    pub fn add(&self, integration: &str, id: &str) -> bool {
        let key = composite_key(integration, id);
        let inserted = self
            .tree
            .insert(&key, &[])
            .map(|prev| prev.is_none())
            .unwrap_or(false);
        let _ = self.tree.flush();
        inserted
    }

    pub fn remove(&self, integration: &str, id: &str) -> bool {
        let key = composite_key(integration, id);
        let removed = self
            .tree
            .remove(&key)
            .map(|prev| prev.is_some())
            .unwrap_or(false);
        let _ = self.tree.flush();
        removed
    }

    /// Returns the new favorite state (true = now favorited).
    pub fn toggle(&self, integration: &str, id: &str) -> bool {
        if self.is_favorite(integration, id) {
            self.remove(integration, id);
            false
        } else {
            self.add(integration, id);
            true
        }
    }

    pub fn list(&self, integration: &str) -> Vec<String> {
        let mut prefix = integration.as_bytes().to_vec();
        prefix.push(KEY_SEP);
        let mut out = Vec::new();
        for kv in self.tree.scan_prefix(&prefix) {
            let Ok((k, _)) = kv else { continue };
            if let Some(rest) = k.strip_prefix(prefix.as_slice()) {
                if let Ok(s) = std::str::from_utf8(rest) {
                    out.push(s.to_string());
                }
            }
        }
        out
    }

    pub fn count(&self, integration: &str) -> usize {
        let mut prefix = integration.as_bytes().to_vec();
        prefix.push(KEY_SEP);
        self.tree.scan_prefix(&prefix).count()
    }
}

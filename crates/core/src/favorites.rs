use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::atomic_write;

/// Per-integration favorites. Keyed by integration name → set of entry IDs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Favorites {
    #[serde(flatten)]
    by_integration: BTreeMap<String, BTreeSet<String>>,
}

impl Favorites {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read favorites {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let parsed: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parse favorites {}", path.display()))?;
        Ok(parsed)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serialize favorites")?;
        atomic_write(path, &body)
    }

    pub fn is_favorite(&self, integration: &str, id: &str) -> bool {
        self.by_integration
            .get(integration)
            .map(|s| s.contains(id))
            .unwrap_or(false)
    }

    pub fn add(&mut self, integration: &str, id: &str) -> bool {
        self.by_integration
            .entry(integration.to_string())
            .or_default()
            .insert(id.to_string())
    }

    pub fn remove(&mut self, integration: &str, id: &str) -> bool {
        self.by_integration
            .get_mut(integration)
            .map(|s| s.remove(id))
            .unwrap_or(false)
    }

    /// Returns the new favorite state (true = now favorited).
    pub fn toggle(&mut self, integration: &str, id: &str) -> bool {
        if self.is_favorite(integration, id) {
            self.remove(integration, id);
            false
        } else {
            self.add(integration, id);
            true
        }
    }

    pub fn list(&self, integration: &str) -> Vec<String> {
        self.by_integration
            .get(integration)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn count(&self, integration: &str) -> usize {
        self.by_integration.get(integration).map(|s| s.len()).unwrap_or(0)
    }
}

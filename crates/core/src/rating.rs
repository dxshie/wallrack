//! Content rating — both as a per-entry value and as the active filter.
//!
//! Native ratings come from WE `project.json` (`contentrating: "Mature" |
//! "Questionable" | "Everyone"`); plain wallpapers carry no native rating.
//! `RatingOverrides` lets the user assign or clear a rating on any entry
//! (mirroring [`TagOverrides`][crate::tags::TagOverrides]), and the
//! `Rating::All` variant doubles as "no filter / unrated" in the picker.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::entry::Index;
use crate::paths::atomic_write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Rating {
    /// Adult / mature content.
    #[value(name = "Mature")]
    Mature,
    /// Borderline content.
    #[value(name = "Questionable")]
    Questionable,
    /// Safe-for-everyone content.
    #[value(name = "Everyone")]
    Everyone,
    /// In filter context: "no filter". In override context: "clear any
    /// rating on this entry".
    #[value(name = "All")]
    All,
}

impl Rating {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mature => "Mature",
            Self::Questionable => "Questionable",
            Self::Everyone => "Everyone",
            Self::All => "All",
        }
    }

    /// Parse a value as stored in state / on entries. Unknown or empty
    /// strings are treated as `All` (== no filter / unrated).
    pub fn parse_state(s: &str) -> Self {
        match s {
            "Mature" => Self::Mature,
            "Questionable" => Self::Questionable,
            "Everyone" => Self::Everyone,
            _ => Self::All,
        }
    }

    /// Cycle order used by the picker keybinding.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Mature,
            Self::Mature => Self::Questionable,
            Self::Questionable => Self::Everyone,
            Self::Everyone => Self::All,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RatingOverrides {
    #[serde(flatten)]
    by_integration: BTreeMap<String, BTreeMap<String, String>>,
}

impl RatingOverrides {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read rating overrides {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let parsed: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parse rating overrides {}", path.display()))?;
        Ok(parsed)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serialize rating overrides")?;
        atomic_write(path, &body)
    }

    /// Pin `id`'s effective rating. `Rating::All` records an explicit
    /// "no rating" — distinct from `clear`, which drops the override
    /// entirely and lets the native rating shine through.
    pub fn set(&mut self, integration: &str, id: &str, rating: Rating) {
        let stored = match rating {
            Rating::All => String::new(),
            r => r.as_str().to_string(),
        };
        self.by_integration
            .entry(integration.to_string())
            .or_default()
            .insert(id.to_string(), stored);
    }

    pub fn clear(&mut self, integration: &str, id: &str) -> bool {
        let removed = self
            .by_integration
            .get_mut(integration)
            .map(|m| m.remove(id).is_some())
            .unwrap_or(false);
        if self.by_integration.get(integration).map(|m| m.is_empty()).unwrap_or(false) {
            self.by_integration.remove(integration);
        }
        removed
    }

    pub fn get(&self, integration: &str, id: &str) -> Option<&str> {
        self.by_integration.get(integration)?.get(id).map(|s| s.as_str())
    }

    pub fn apply_to(&self, idx: &mut Index) {
        let Some(by_id) = self.by_integration.get(&idx.integration) else { return };
        for entry in &mut idx.entries {
            if let Some(rating) = by_id.get(&entry.id) {
                entry.rating = rating.clone();
            }
        }
    }
}

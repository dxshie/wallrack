//! Content rating — both as a per-entry value and as the active filter.
//!
//! Native ratings come from WE `project.json` (`contentrating: "Mature" |
//! "Questionable" | "Everyone"`); plain wallpapers carry no native rating.
//! `RatingOverrides` lets the user assign or clear a rating on any entry,
//! and the `Rating::All` variant doubles as "no filter / unrated" in the
//! picker.

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sled::{Db, Tree};

use crate::entry::Index;
use crate::store::{TREE_RATING_OVERRIDES, composite_key};

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

pub struct RatingOverrides {
    tree: Tree,
}

impl RatingOverrides {
    pub fn open(db: &Db) -> Result<Self> {
        let tree = db
            .open_tree(TREE_RATING_OVERRIDES)
            .with_context(|| format!("open sled tree `{TREE_RATING_OVERRIDES}`"))?;
        Ok(Self { tree })
    }

    /// Pin `id`'s effective rating. `Rating::All` records an explicit
    /// "no rating" — distinct from `clear`, which drops the override
    /// entirely and lets the native rating shine through.
    pub fn set(&self, integration: &str, id: &str, rating: Rating) -> Result<()> {
        let stored = match rating {
            Rating::All => "",
            r => r.as_str(),
        };
        let key = composite_key(integration, id);
        self.tree.insert(&key, stored.as_bytes())?;
        self.tree.flush()?;
        Ok(())
    }

    pub fn clear(&self, integration: &str, id: &str) -> Result<bool> {
        let key = composite_key(integration, id);
        let removed = self.tree.remove(&key)?.is_some();
        self.tree.flush()?;
        Ok(removed)
    }

    pub fn get(&self, integration: &str, id: &str) -> Option<String> {
        let key = composite_key(integration, id);
        let ivec = self.tree.get(&key).ok().flatten()?;
        std::str::from_utf8(&ivec).ok().map(|s| s.to_string())
    }

    pub fn apply_to(&self, idx: &mut Index) {
        for entry in &mut idx.entries {
            let key = composite_key(&idx.integration, entry.id());
            if let Ok(Some(bytes)) = self.tree.get(&key) {
                if let Ok(s) = std::str::from_utf8(&bytes) {
                    entry.set_rating(s.to_string());
                }
            }
        }
    }
}

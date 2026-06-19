//! Per-monitor "currently applied" state — sled-backed source of truth for
//! which integration owns which monitor and what target is on it.
//!
//! This is what makes the multi-integration display dance work:
//!
//! - Applying a non-WE integration to a monitor that's currently owned by
//!   `we` releases that monitor from the running `linux-wallpaperengine`
//!   process (otherwise the WE overlay stays on top of the new wallpaper).
//! - Applying `we` to N monitors composes a single `linux-wallpaperengine`
//!   invocation with one `--screen-root M --bg ID` pair per monitor, so
//!   different workshop wallpapers can coexist on different screens.
//! - `wallrack applied restore` reads this and re-applies every monitor on
//!   WM/DE startup, batching WE into one process.
//!
//! Sled tree key: monitor name. Value: JSON [`AppliedEntry`].

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sled::{Db, Tree};

use crate::store::TREE_APPLIED;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedEntry {
    pub integration: String,
    /// Integration-specific target string. For `wallpaper`/`we_image` this is
    /// the entry id (image path); for `we` it's the workshop id (so we can
    /// reconstruct `--bg <id>` without re-resolving the index).
    pub target: String,
}

pub struct Applied {
    tree: Tree,
}

impl Applied {
    pub fn open(db: &Db) -> Result<Self> {
        let tree = db
            .open_tree(TREE_APPLIED)
            .with_context(|| format!("open sled tree `{TREE_APPLIED}`"))?;
        Ok(Self { tree })
    }

    pub fn set(&self, monitor: &str, integration: &str, target: &str) -> Result<()> {
        let entry = AppliedEntry {
            integration: integration.to_string(),
            target: target.to_string(),
        };
        let body = serde_json::to_vec(&entry).context("serialize applied entry")?;
        self.tree.insert(monitor.as_bytes(), body)?;
        self.tree.flush()?;
        Ok(())
    }

    pub fn get(&self, monitor: &str) -> Option<AppliedEntry> {
        let bytes = self.tree.get(monitor.as_bytes()).ok().flatten()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn remove(&self, monitor: &str) -> Result<bool> {
        let was = self.tree.remove(monitor.as_bytes())?.is_some();
        self.tree.flush()?;
        Ok(was)
    }

    /// Every (monitor → entry) currently tracked, ordered by monitor name.
    /// Skips sled migration sentinels.
    pub fn all(&self) -> BTreeMap<String, AppliedEntry> {
        let mut out = BTreeMap::new();
        for kv in self.tree.iter() {
            let Ok((k, v)) = kv else { continue };
            let Ok(key) = std::str::from_utf8(&k) else {
                continue;
            };
            if key.starts_with("__") {
                continue;
            }
            let Ok(entry) = serde_json::from_slice::<AppliedEntry>(&v) else {
                continue;
            };
            out.insert(key.to_string(), entry);
        }
        out
    }

    /// Just the (monitor → target) pairs whose entry is owned by `integration`.
    /// Useful for composing the WE multi-monitor command line.
    pub fn by_integration(&self, integration: &str) -> BTreeMap<String, String> {
        self.all()
            .into_iter()
            .filter_map(|(mon, e)| (e.integration == integration).then_some((mon, e.target)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> Db {
        let dir = std::env::temp_dir().join(format!(
            "wallrack-applied-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        sled::Config::new()
            .path(dir)
            .temporary(true)
            .open()
            .unwrap()
    }

    #[test]
    fn set_then_get_roundtrips() {
        let db = temp_db();
        let a = Applied::open(&db).unwrap();
        a.set("DP-1", "we", "1234567890").unwrap();
        let got = a.get("DP-1").unwrap();
        assert_eq!(got.integration, "we");
        assert_eq!(got.target, "1234567890");
    }

    #[test]
    fn by_integration_filters() {
        let db = temp_db();
        let a = Applied::open(&db).unwrap();
        a.set("DP-1", "we", "1111").unwrap();
        a.set("DP-2", "we", "2222").unwrap();
        a.set("HDMI-A-1", "wallpaper", "/img.jpg").unwrap();
        let we = a.by_integration("we");
        assert_eq!(we.len(), 2);
        assert_eq!(we.get("DP-1"), Some(&"1111".to_string()));
        assert_eq!(we.get("DP-2"), Some(&"2222".to_string()));
        assert!(!we.contains_key("HDMI-A-1"));
    }

    #[test]
    fn remove_clears_entry() {
        let db = temp_db();
        let a = Applied::open(&db).unwrap();
        a.set("DP-1", "we", "1111").unwrap();
        assert!(a.remove("DP-1").unwrap());
        assert!(a.get("DP-1").is_none());
        // Removing a missing key returns false.
        assert!(!a.remove("DP-1").unwrap());
    }
}

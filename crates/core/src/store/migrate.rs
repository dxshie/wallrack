//! One-shot migration from the pre-0.3 JSON files into sled. Runs every
//! time the store is opened; the `sentinel` keys make it a no-op after the
//! first successful import.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use sled::{Db, IVec};

use super::{
    TREE_APPLIED, TREE_FAVORITES, TREE_RATING_OVERRIDES, TREE_STATE, TREE_TAG_CATALOG,
    TREE_TAG_OVERRIDES, composite_key, write_json,
};

const SENTINEL: &str = "__migrated_from_json";

pub fn run(cache_root: &Path, db: &Db) -> Result<()> {
    migrate_one(
        cache_root.join("favorites.json").as_path(),
        db.open_tree(TREE_FAVORITES)?,
        import_favorites,
    )?;
    migrate_one(
        cache_root.join("tags.json").as_path(),
        db.open_tree(TREE_TAG_OVERRIDES)?,
        import_tag_overrides,
    )?;
    migrate_one(
        cache_root.join("tag_catalog.json").as_path(),
        db.open_tree(TREE_TAG_CATALOG)?,
        import_tag_catalog,
    )?;
    migrate_one(
        cache_root.join("rating_overrides.json").as_path(),
        db.open_tree(TREE_RATING_OVERRIDES)?,
        import_rating_overrides,
    )?;
    migrate_one(
        cache_root.join("state.json").as_path(),
        db.open_tree(TREE_STATE)?,
        import_state,
    )?;
    migrate_one(
        cache_root.join("we").join("monitor-state.json").as_path(),
        db.open_tree(TREE_APPLIED)?,
        import_we_monitor_state,
    )?;
    Ok(())
}

fn migrate_one(
    json_path: &Path,
    tree: sled::Tree,
    import: impl FnOnce(&sled::Tree, &str) -> Result<()>,
) -> Result<()> {
    if tree.contains_key(SENTINEL)? {
        return Ok(());
    }
    if !json_path.is_file() {
        tree.insert(SENTINEL, IVec::from(b"1"))?;
        tree.flush()?;
        return Ok(());
    }
    let raw = std::fs::read_to_string(json_path)?;
    if !raw.trim().is_empty() {
        import(&tree, &raw)?;
    }
    tree.insert(SENTINEL, IVec::from(b"1"))?;
    tree.flush()?;
    // Rename the legacy file rather than delete so the user can recover
    // manually if anything looks wrong post-migration.
    let backup = json_path.with_extension("json.pre-sled");
    let _ = std::fs::rename(json_path, &backup);
    log::info!(
        "wallrack: migrated {} into sled (backup at {})",
        json_path.display(),
        backup.display()
    );
    Ok(())
}

// ─── per-type legacy importers ──────────────────────────────────────────

fn import_favorites(tree: &sled::Tree, raw: &str) -> Result<()> {
    let parsed: BTreeMap<String, BTreeSet<String>> = serde_json::from_str(raw)?;
    for (integration, ids) in parsed {
        for id in ids {
            tree.insert(composite_key(&integration, &id), &[])?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct LegacyEntryOverride {
    #[serde(default)]
    added: BTreeSet<String>,
    #[serde(default)]
    removed: BTreeSet<String>,
}

fn import_tag_overrides(tree: &sled::Tree, raw: &str) -> Result<()> {
    let parsed: BTreeMap<String, BTreeMap<String, LegacyEntryOverride>> =
        serde_json::from_str(raw)?;
    for (integration, by_id) in parsed {
        for (id, ov) in by_id {
            let key = composite_key(&integration, &id);
            let value = serde_json::json!({
                "added": ov.added,
                "removed": ov.removed,
            });
            write_json(tree, &key, &value)?;
        }
    }
    Ok(())
}

fn import_tag_catalog(tree: &sled::Tree, raw: &str) -> Result<()> {
    let parsed: BTreeMap<String, BTreeSet<String>> = serde_json::from_str(raw)?;
    for (integration, tags) in parsed {
        for t in tags {
            tree.insert(composite_key(&integration, &t), &[])?;
        }
    }
    Ok(())
}

fn import_rating_overrides(tree: &sled::Tree, raw: &str) -> Result<()> {
    let parsed: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(raw)?;
    for (integration, by_id) in parsed {
        for (id, rating) in by_id {
            tree.insert(composite_key(&integration, &id), rating.as_bytes())?;
        }
    }
    Ok(())
}

fn import_state(tree: &sled::Tree, raw: &str) -> Result<()> {
    let parsed: BTreeMap<String, String> = serde_json::from_str(raw)?;
    for (k, v) in parsed {
        tree.insert(k.as_bytes(), v.as_bytes())?;
    }
    Ok(())
}

/// Legacy WE-only `we/monitor-state.json` was `{ "<monitor>": "<workshop_id>" }`.
/// The new `applied` tree generalizes that: every integration writes its
/// (monitor → target) here, so the schema gains an `integration` discriminator.
/// Pre-existing entries all came from the WE integration.
fn import_we_monitor_state(tree: &sled::Tree, raw: &str) -> Result<()> {
    let parsed: BTreeMap<String, String> = serde_json::from_str(raw)?;
    for (monitor, workshop_id) in parsed {
        let body = serde_json::json!({
            "integration": "we",
            "target": workshop_id,
        });
        write_json(tree, monitor.as_bytes(), &body)?;
    }
    Ok(())
}

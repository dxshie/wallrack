use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::store;

pub struct Paths {
    cache_root: PathBuf,
    config_root: PathBuf,
    store: sled::Db,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let cache_root = dirs::cache_dir()
            .ok_or_else(|| anyhow!("no XDG cache dir"))?
            .join("wallrack");
        let config_root = dirs::config_dir()
            .ok_or_else(|| anyhow!("no XDG config dir"))?
            .join("wallrack");
        // Sled wants the cache root to exist before we open the DB inside it.
        fs::create_dir_all(&cache_root)
            .with_context(|| format!("create cache dir {}", cache_root.display()))?;
        let store = store::open(&cache_root)?;
        Ok(Self {
            cache_root,
            config_root,
            store,
        })
    }

    /// Sled handle — opens the trees backing favorites, tags, ratings, state.
    pub fn store(&self) -> &sled::Db {
        &self.store
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_root.join("config.toml")
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_root
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_root
    }

    pub fn integration_dir(&self, integration: &str) -> PathBuf {
        self.cache_root.join(integration)
    }

    pub fn index_file(&self, integration: &str) -> PathBuf {
        self.integration_dir(integration).join("index.json")
    }

    pub fn thumbs_dir(&self, integration: &str) -> PathBuf {
        self.integration_dir(integration).join("thumbs")
    }

    pub fn favorites_file(&self) -> PathBuf {
        self.cache_root.join("favorites.json")
    }

    pub fn tags_file(&self) -> PathBuf {
        self.cache_root.join("tags.json")
    }

    pub fn tag_catalog_file(&self) -> PathBuf {
        self.cache_root.join("tag_catalog.json")
    }

    pub fn rating_overrides_file(&self) -> PathBuf {
        self.cache_root.join("rating_overrides.json")
    }

    pub fn state_file(&self) -> PathBuf {
        self.cache_root.join("state.json")
    }

    pub fn daemon_pid_file(&self) -> PathBuf {
        self.cache_root.join("daemon.pid")
    }

    pub fn we_monitor_state_file(&self) -> PathBuf {
        self.cache_root.join("we").join("monitor-state.json")
    }

    pub fn ensure_cache(&self) -> Result<()> {
        fs::create_dir_all(&self.cache_root)
            .with_context(|| format!("create cache dir {}", self.cache_root.display()))?;
        Ok(())
    }

    pub fn ensure_integration(&self, integration: &str) -> Result<()> {
        let dir = self.integration_dir(integration);
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let thumbs = self.thumbs_dir(integration);
        fs::create_dir_all(&thumbs).with_context(|| format!("create {}", thumbs.display()))?;
        Ok(())
    }

    pub fn ensure_config(&self) -> Result<()> {
        fs::create_dir_all(&self.config_root)
            .with_context(|| format!("create config dir {}", self.config_root.display()))?;
        Ok(())
    }
}

/// Expand a leading `~/` to the user's home directory.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

/// Atomically write `data` to `path` via tempfile + rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("wallrack"),
        std::process::id()
    ));
    fs::write(&tmp, data).with_context(|| format!("write tmp {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn expand_home_replaces_leading_tilde_slash() {
        if let Some(home) = dirs::home_dir() {
            let expanded = expand_home("~/Pictures/booru");
            assert_eq!(expanded, home.join("Pictures/booru"));
        }
    }

    #[test]
    fn expand_home_leaves_non_tilde_paths_untouched() {
        assert_eq!(expand_home("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_home("relative/x"), PathBuf::from("relative/x"));
        // A bare `~` (no slash) is not the tilde-slash prefix and stays verbatim.
        assert_eq!(expand_home("~user/x"), PathBuf::from("~user/x"));
    }

    fn unique_tmp(suffix: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        env::temp_dir().join(format!(
            "wallrack-atomic-{}-{stamp}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn atomic_write_creates_parents_and_writes_payload() {
        let dir = unique_tmp("a");
        let target = dir.join("nested").join("out.bin");
        atomic_write(&target, b"hello").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hello");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let dir = unique_tmp("b");
        let target = dir.join("out.bin");
        atomic_write(&target, b"first").unwrap();
        atomic_write(&target, b"second").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"second");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_behind_on_success() {
        let dir = unique_tmp("c");
        let target = dir.join("out.bin");
        atomic_write(&target, b"x").unwrap();
        let leftover: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .filter(|n| n.to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftover.is_empty(), "found stray tmpfiles: {leftover:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}

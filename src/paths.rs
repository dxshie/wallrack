use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

pub struct Paths {
    cache_root: PathBuf,
    config_root: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let cache_root = dirs::cache_dir()
            .ok_or_else(|| anyhow!("no XDG cache dir"))?
            .join("wallrack");
        let config_root = dirs::config_dir()
            .ok_or_else(|| anyhow!("no XDG config dir"))?
            .join("wallrack");
        Ok(Self { cache_root, config_root })
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
        fs::create_dir_all(&dir)
            .with_context(|| format!("create {}", dir.display()))?;
        let thumbs = self.thumbs_dir(integration);
        fs::create_dir_all(&thumbs)
            .with_context(|| format!("create {}", thumbs.display()))?;
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
    let parent = path.parent().ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create {}", parent.display()))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("wallrack"),
        std::process::id()
    ));
    fs::write(&tmp, data)
        .with_context(|| format!("write tmp {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

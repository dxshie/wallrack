use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{Paths, expand_home};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub thumbnails: Thumbnails,
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    #[serde(default)]
    pub wallpaper_engine_image: WallpaperEngineImageConfig,
    #[serde(default)]
    pub wallpaper_engine: WallpaperEngineConfig,
    #[serde(default)]
    pub booru: BooruConfig,
    #[serde(default)]
    pub hooks: Hooks,
}

/// Shell commands run around every successful `wallrack apply`. Both run via
/// `sh -c` with `WALLRACK_WALLPAPER`, `WALLRACK_MONITOR`, and
/// `WALLRACK_INTEGRATION` set. Non-zero exit prints a warning but does not
/// fail the apply itself. Leave empty to skip.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hooks {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pre_apply_hook: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub post_apply_hook: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thumbnails {
    #[serde(default = "default_thumb_size")]
    pub size: u32,
}

/// Per-integration commands. Templates use `{{image}}`, `{{monitor}}`,
/// `{{folder}}`, `{{workshop_id}}` placeholders — substituted as plain text
/// and passed to `sh -c`, so users are responsible for quoting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Sets a wallpaper. Receives `{{image}}` / `{{folder}}` / `{{workshop_id}}`
    /// and `{{monitor}}` depending on the integration.
    #[serde(default)]
    pub apply_cmd: Option<String>,
    /// Lists monitors — must print one monitor name per line.
    #[serde(default)]
    pub monitors_cmd: Option<String>,
    /// Optional. Prints currently-displayed wallpapers as
    /// `<monitor>\t<path>` lines. When unset, the monitor picker simply
    /// doesn't show current-wallpaper thumbs.
    #[serde(default)]
    pub current_image_cmd: Option<String>,
}

/// Plain wallpaper images from user-provided directories.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WallpaperConfig {
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub backend: BackendConfig,
}

/// Wallpaper Engine workshop folders scraped for image assets — applies them
/// like a normal image wallpaper (not via linux-wallpaperengine).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WallpaperEngineImageConfig {
    /// Workshop dir. When unset, falls back to `wallpaper_engine.workshop_dir`
    /// since the typical setup uses the same source for both.
    #[serde(default)]
    pub workshop_dir: Option<String>,
    #[serde(default)]
    pub backend: BackendConfig,
}

/// Wallpaper Engine projects, applied via linux-wallpaperengine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperEngineConfig {
    #[serde(default = "default_we_workshop_dir")]
    pub workshop_dir: String,
    #[serde(default)]
    pub backend: BackendConfig,
}

impl Default for WallpaperEngineConfig {
    fn default() -> Self {
        Self { workshop_dir: default_we_workshop_dir(), backend: BackendConfig::default() }
    }
}

impl Default for Thumbnails {
    fn default() -> Self {
        Self { size: default_thumb_size() }
    }
}

fn default_thumb_size() -> u32 { 256 }

fn default_we_workshop_dir() -> String {
    "~/.local/share/Steam/steamapps/workshop/content/431960".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            thumbnails: Thumbnails::default(),
            wallpaper: WallpaperConfig::default(),
            wallpaper_engine_image: WallpaperEngineImageConfig::default(),
            wallpaper_engine: WallpaperEngineConfig::default(),
            booru: BooruConfig::default(),
            hooks: Hooks::default(),
        }
    }
}

// ─── booru integration ──────────────────────────────────────────────────────
// Search-driven integration that talks to danbooru-style image boards
// (konachan, yandere, danbooru, gelbooru, …). Search results are cached as
// the integration's index; "applying" an entry downloads it into
// `download_dir`.

/// API shape of the target booru. Field naming differs enough across the three
/// big families that we dispatch on this rather than papering over with serde
/// aliases. Defaults to moebooru — the family konachan/yande.re belong to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BooruApiKind {
    /// konachan, yande.re — `/post.json?tags=…&page=1`
    Moebooru,
    /// danbooru.donmai.us — `/posts.json?tags=…&page=1`
    Danbooru,
    /// gelbooru, safebooru — `/index.php?page=dapi&s=post&q=index&json=1&tags=…&pid=0`
    Gelbooru,
}

impl Default for BooruApiKind {
    fn default() -> Self { BooruApiKind::Moebooru }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BooruSite {
    pub base_url: String,
    #[serde(default)]
    pub api_kind: BooruApiKind,
    /// Login / username. Required for moebooru (with `password` or
    /// `password_hash`) and danbooru (with `api_key`). Gelbooru ignores this
    /// — it auths via `user_id`/`api_key` pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// API key. Required for danbooru/gelbooru auth. Ignored by moebooru.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Numeric user_id, required for gelbooru auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Plaintext password (moebooru only). Hashed at request time using
    /// `password_salt` — never sent in the clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Pre-computed `password_hash` (moebooru only). Use this when you'd
    /// rather not store the plaintext password in config.toml; copy it from
    /// your browser's saved cookies or compute it once externally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    /// Moebooru password salt template — the literal `{}` is replaced with
    /// the password and the result is SHA1'd. Built-in defaults cover
    /// konachan and yande.re; custom moebooru instances need their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_salt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BooruConfig {
    /// Where downloaded full-size images are placed. Set this to one of your
    /// `wallpaper.dirs` if you want them to appear in the wallpaper picker
    /// after a re-index.
    #[serde(default = "default_booru_download_dir")]
    pub download_dir: String,
    /// Site key to use when `--site` is not passed.
    #[serde(default = "default_booru_default_site")]
    pub default_site: String,
    /// Results per page on `wallrack booru search`. Most boorus cap this at
    /// ~100; konachan/yandere default to 21 in the web UI.
    #[serde(default = "default_booru_per_page")]
    pub per_page: u32,
    /// Per-request timeout for booru search API calls, in seconds. Applies
    /// to both connect and read. A flaky network on a slow site can stall
    /// rofi for the full duration — keep this short enough that the user
    /// notices the retry, long enough that healthy queries succeed.
    #[serde(default = "default_booru_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Max retry attempts after the initial search request fails on a
    /// transient error (network/timeout, 5xx, 429). 0 disables retries.
    /// 4xx other than 429 are never retried — those mean "bad query" and
    /// will fail the same way every time.
    #[serde(default = "default_booru_max_retries")]
    pub max_retries: u32,
    /// Initial backoff between retries, in milliseconds. Doubled on each
    /// subsequent attempt (capped at 30s) for a simple exponential.
    #[serde(default = "default_booru_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
    /// Configured sites. Empty maps fall back to the built-in defaults
    /// (`konachan`, `yandere`, `danbooru`, `gelbooru`, `safebooru`).
    #[serde(default)]
    pub sites: BTreeMap<String, BooruSite>,
}

impl Default for BooruConfig {
    fn default() -> Self {
        Self {
            download_dir: default_booru_download_dir(),
            default_site: default_booru_default_site(),
            per_page: default_booru_per_page(),
            request_timeout_secs: default_booru_request_timeout_secs(),
            max_retries: default_booru_max_retries(),
            retry_backoff_ms: default_booru_retry_backoff_ms(),
            sites: BTreeMap::new(),
        }
    }
}

/// Resolved HTTP policy for booru search requests. Built from `BooruConfig`
/// at call time so per-site overrides (future work) can layer cleanly.
#[derive(Debug, Clone, Copy)]
pub struct BooruHttpPolicy {
    pub timeout: std::time::Duration,
    pub max_retries: u32,
    pub retry_backoff: std::time::Duration,
}

impl BooruConfig {
    pub fn http_policy(&self) -> BooruHttpPolicy {
        BooruHttpPolicy {
            timeout: std::time::Duration::from_secs(self.request_timeout_secs.max(1)),
            max_retries: self.max_retries,
            retry_backoff: std::time::Duration::from_millis(self.retry_backoff_ms),
        }
    }
}

impl BooruConfig {
    pub fn download_dir(&self) -> PathBuf {
        expand_home(&self.download_dir)
    }

    /// User-configured sites merged with built-in defaults. User-set fields
    /// win; unset fields fall back to the builtin. This matters for
    /// `password_salt` in particular — users overriding `[booru.sites.konachan]`
    /// shouldn't have to re-specify the moebooru salt to keep auth working.
    pub fn resolved_sites(&self) -> BTreeMap<String, BooruSite> {
        let mut sites = builtin_booru_sites();
        for (k, user) in &self.sites {
            let merged = match sites.remove(k) {
                Some(builtin) => BooruSite {
                    base_url: if user.base_url.is_empty() { builtin.base_url } else { user.base_url.clone() },
                    api_kind: user.api_kind,
                    login: user.login.clone().or(builtin.login),
                    api_key: user.api_key.clone().or(builtin.api_key),
                    user_id: user.user_id.clone().or(builtin.user_id),
                    password: user.password.clone().or(builtin.password),
                    password_hash: user.password_hash.clone().or(builtin.password_hash),
                    password_salt: user.password_salt.clone().or(builtin.password_salt),
                },
                None => user.clone(),
            };
            sites.insert(k.clone(), merged);
        }
        sites
    }

    pub fn resolve_site(&self, key: &str) -> Option<BooruSite> {
        self.resolved_sites().get(key).cloned()
    }
}

fn default_booru_download_dir() -> String { "~/Pictures/booru".to_string() }
fn default_booru_default_site() -> String { "konachan".to_string() }
fn default_booru_per_page() -> u32 { 20 }
fn default_booru_request_timeout_secs() -> u64 { 30 }
fn default_booru_max_retries() -> u32 { 2 }
fn default_booru_retry_backoff_ms() -> u64 { 500 }

fn builtin_booru_sites() -> BTreeMap<String, BooruSite> {
    let mut m = BTreeMap::new();
    // Moebooru salt templates are site-specific and load-bearing for login —
    // the API rejects a hash built with the wrong salt. Values are the
    // published constants from each site's source.
    m.insert("konachan".into(), BooruSite {
        base_url: "https://konachan.com".into(),
        api_kind: BooruApiKind::Moebooru,
        login: None, api_key: None, user_id: None,
        password: None, password_hash: None,
        password_salt: Some("So-I-Heard_You_Like_Mupkids.{}--".into()),
    });
    m.insert("yandere".into(), BooruSite {
        base_url: "https://yande.re".into(),
        api_kind: BooruApiKind::Moebooru,
        login: None, api_key: None, user_id: None,
        password: None, password_hash: None,
        password_salt: Some("choujin-steiner--{}--".into()),
    });
    m.insert("danbooru".into(), BooruSite {
        base_url: "https://danbooru.donmai.us".into(),
        api_kind: BooruApiKind::Danbooru,
        login: None, api_key: None, user_id: None,
        password: None, password_hash: None, password_salt: None,
    });
    m.insert("gelbooru".into(), BooruSite {
        base_url: "https://gelbooru.com".into(),
        api_kind: BooruApiKind::Gelbooru,
        login: None, api_key: None, user_id: None,
        password: None, password_hash: None, password_salt: None,
    });
    m.insert("safebooru".into(), BooruSite {
        base_url: "https://safebooru.org".into(),
        api_kind: BooruApiKind::Gelbooru,
        login: None, api_key: None, user_id: None,
        password: None, password_hash: None, password_salt: None,
    });
    m
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.config_file();
        if !file.exists() {
            paths.ensure_config()?;
            let default = Config::default();
            let body = toml::to_string_pretty(&default).context("serialize default config")?;
            fs::write(&file, body)
                .with_context(|| format!("write default config to {}", file.display()))?;
            return Ok(default);
        }
        let body = fs::read_to_string(&file)
            .with_context(|| format!("read config {}", file.display()))?;
        let cfg: Config = toml::from_str(&body)
            .with_context(|| format!("parse config {}", file.display()))?;
        Ok(cfg)
    }

    pub fn wallpaper_dirs(&self) -> Vec<PathBuf> {
        self.wallpaper.dirs.iter().map(|s| expand_home(s)).collect()
    }

    /// Workshop dir for the we_image integration. Falls back to the WE
    /// workshop dir when not explicitly set — the typical setup points both
    /// at the same Steam workshop content folder.
    pub fn we_image_workshop_dir(&self) -> PathBuf {
        match self.wallpaper_engine_image.workshop_dir.as_deref() {
            Some(d) => expand_home(d),
            None => self.we_workshop_dir(),
        }
    }

    pub fn we_workshop_dir(&self) -> PathBuf {
        expand_home(&self.wallpaper_engine.workshop_dir)
    }
}

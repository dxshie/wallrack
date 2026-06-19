//! Booru integration config — site definitions, default HTTP policy, and
//! built-in sites. Lives in its own module because (1) it's the bulk of the
//! config types and (2) it carries its own concept set (api kinds, auth,
//! retry policy) that doesn't apply to the other integrations.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::paths::expand_home;

/// API shape of the target booru. Field naming differs enough across the three
/// big families that we dispatch on this rather than papering over with serde
/// aliases. Defaults to moebooru — the family konachan/yande.re belong to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BooruApiKind {
    /// konachan, yande.re — `/post.json?tags=…&page=1`
    #[default]
    Moebooru,
    /// danbooru.donmai.us — `/posts.json?tags=…&page=1`
    Danbooru,
    /// gelbooru, safebooru — `/index.php?page=dapi&s=post&q=index&json=1&tags=…&pid=0`
    Gelbooru,
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
    #[serde(default = "default_download_dir")]
    pub download_dir: String,
    /// Site key to use when `--site` is not passed.
    #[serde(default = "default_default_site")]
    pub default_site: String,
    /// Results per page on `wallrack booru search`. Most boorus cap this at
    /// ~100; konachan/yandere default to 21 in the web UI.
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// Per-request timeout for booru search API calls, in seconds. Applies
    /// to both connect and read. A flaky network on a slow site can stall
    /// rofi for the full duration — keep this short enough that the user
    /// notices the retry, long enough that healthy queries succeed.
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Max retry attempts after the initial search request fails on a
    /// transient error (network/timeout, 5xx, 429). 0 disables retries.
    /// 4xx other than 429 are never retried — those mean "bad query" and
    /// will fail the same way every time.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Initial backoff between retries, in milliseconds. Doubled on each
    /// subsequent attempt (capped at 30s) for a simple exponential.
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
    /// Configured sites. Empty maps fall back to the built-in defaults
    /// (`konachan`, `yandere`, `danbooru`, `gelbooru`, `safebooru`).
    #[serde(default)]
    pub sites: BTreeMap<String, BooruSite>,
}

impl Default for BooruConfig {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            default_site: default_default_site(),
            per_page: default_per_page(),
            request_timeout_secs: default_request_timeout_secs(),
            max_retries: default_max_retries(),
            retry_backoff_ms: default_retry_backoff_ms(),
            sites: BTreeMap::new(),
        }
    }
}

/// Resolved HTTP policy for booru search requests. Built from `BooruConfig`
/// at call time so per-site overrides (future work) can layer cleanly.
#[derive(Debug, Clone, Copy)]
pub struct BooruHttpPolicy {
    pub timeout: Duration,
    pub max_retries: u32,
    pub retry_backoff: Duration,
}

impl BooruConfig {
    pub fn http_policy(&self) -> BooruHttpPolicy {
        BooruHttpPolicy {
            timeout: Duration::from_secs(self.request_timeout_secs.max(1)),
            max_retries: self.max_retries,
            retry_backoff: Duration::from_millis(self.retry_backoff_ms),
        }
    }

    pub fn download_dir(&self) -> PathBuf {
        expand_home(&self.download_dir)
    }

    /// User-configured sites merged with built-in defaults. User-set fields
    /// win; unset fields fall back to the builtin. This matters for
    /// `password_salt` in particular — users overriding `[booru.sites.konachan]`
    /// shouldn't have to re-specify the moebooru salt to keep auth working.
    pub fn resolved_sites(&self) -> BTreeMap<String, BooruSite> {
        let mut sites = builtin_sites();
        for (k, user) in &self.sites {
            let merged = match sites.remove(k) {
                Some(builtin) => BooruSite {
                    base_url: if user.base_url.is_empty() {
                        builtin.base_url
                    } else {
                        user.base_url.clone()
                    },
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

fn default_download_dir() -> String {
    "~/Pictures/booru".to_string()
}
fn default_default_site() -> String {
    "konachan".to_string()
}
fn default_per_page() -> u32 {
    20
}
fn default_request_timeout_secs() -> u64 {
    30
}
fn default_max_retries() -> u32 {
    2
}
fn default_retry_backoff_ms() -> u64 {
    500
}

fn builtin_sites() -> BTreeMap<String, BooruSite> {
    let mut m = BTreeMap::new();
    // Moebooru salt templates are site-specific and load-bearing for login —
    // the API rejects a hash built with the wrong salt. Values are the
    // published constants from each site's source.
    m.insert(
        "konachan".into(),
        BooruSite {
            base_url: "https://konachan.com".into(),
            api_kind: BooruApiKind::Moebooru,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: Some("So-I-Heard_You_Like_Mupkids.{}--".into()),
        },
    );
    m.insert(
        "yandere".into(),
        BooruSite {
            base_url: "https://yande.re".into(),
            api_kind: BooruApiKind::Moebooru,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: Some("choujin-steiner--{}--".into()),
        },
    );
    m.insert(
        "danbooru".into(),
        BooruSite {
            base_url: "https://danbooru.donmai.us".into(),
            api_kind: BooruApiKind::Danbooru,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: None,
        },
    );
    m.insert(
        "gelbooru".into(),
        BooruSite {
            base_url: "https://gelbooru.com".into(),
            api_kind: BooruApiKind::Gelbooru,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: None,
        },
    );
    m.insert(
        "safebooru".into(),
        BooruSite {
            base_url: "https://safebooru.org".into(),
            api_kind: BooruApiKind::Gelbooru,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: None,
        },
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_user_site() -> BooruSite {
        BooruSite {
            base_url: String::new(),
            api_kind: BooruApiKind::Moebooru,
            login: None,
            api_key: None,
            user_id: None,
            password: None,
            password_hash: None,
            password_salt: None,
        }
    }

    #[test]
    fn defaults_expose_all_builtin_sites() {
        let sites = BooruConfig::default().resolved_sites();
        for key in ["konachan", "yandere", "danbooru", "gelbooru", "safebooru"] {
            assert!(sites.contains_key(key), "missing builtin {key}");
        }
    }

    #[test]
    fn user_login_overlays_onto_builtin_konachan_keeps_salt() {
        let mut cfg = BooruConfig::default();
        let mut user = empty_user_site();
        user.login = Some("alice".into());
        user.password = Some("hunter2".into());
        cfg.sites.insert("konachan".into(), user);
        let resolved = cfg.resolve_site("konachan").unwrap();
        assert_eq!(resolved.login.as_deref(), Some("alice"));
        assert_eq!(resolved.password.as_deref(), Some("hunter2"));
        // Builtin base_url survives because user left it blank.
        assert_eq!(resolved.base_url, "https://konachan.com");
        assert!(
            resolved
                .password_salt
                .as_deref()
                .unwrap_or("")
                .contains("Mupkids"),
            "builtin moebooru salt should still be present"
        );
    }

    #[test]
    fn user_base_url_overrides_builtin_when_set() {
        let mut cfg = BooruConfig::default();
        let mut user = empty_user_site();
        user.base_url = "https://mirror.konachan.example".into();
        cfg.sites.insert("konachan".into(), user);
        assert_eq!(
            cfg.resolve_site("konachan").unwrap().base_url,
            "https://mirror.konachan.example"
        );
    }

    #[test]
    fn unknown_user_site_is_added_alongside_builtins() {
        let mut cfg = BooruConfig::default();
        let mut user = empty_user_site();
        user.base_url = "https://custom.example".into();
        user.api_kind = BooruApiKind::Danbooru;
        cfg.sites.insert("custom".into(), user);
        let resolved = cfg.resolved_sites();
        assert!(resolved.contains_key("konachan"));
        let custom = resolved.get("custom").unwrap();
        assert_eq!(custom.base_url, "https://custom.example");
        assert_eq!(custom.api_kind, BooruApiKind::Danbooru);
    }

    #[test]
    fn http_policy_clamps_timeout_to_at_least_one_second() {
        let cfg = BooruConfig {
            request_timeout_secs: 0,
            ..BooruConfig::default()
        };
        assert_eq!(cfg.http_policy().timeout.as_secs(), 1);
    }

    #[test]
    fn http_policy_carries_configured_retry_settings() {
        let cfg = BooruConfig {
            max_retries: 5,
            retry_backoff_ms: 750,
            ..BooruConfig::default()
        };
        let p = cfg.http_policy();
        assert_eq!(p.max_retries, 5);
        assert_eq!(p.retry_backoff.as_millis(), 750);
    }
}

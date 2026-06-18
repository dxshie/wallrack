//! Booru integration — danbooru-style image board search and download.
//!
//! Unlike the other integrations, the booru integration's index is *not*
//! built by scanning a directory: it's whatever the last `wallrack booru
//! search` call returned. The index file is overwritten on every search, so
//! pagination is "the current page is the index".
//!
//! "Applying" a booru entry downloads the full-size image into
//! `[booru].download_dir` (defaulting to `~/Pictures/booru`). The monitor
//! argument is ignored — there is no real monitor for a download operation,
//! but `merged_backend()` advertises a single fake `download` monitor so the
//! standard `wallrack monitors` → `wallrack apply` flow still works.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::config::{BackendConfig, BooruApiKind, BooruHttpPolicy, BooruSite, Config};
use crate::entry::{Entry, Index};
use crate::integrations::Integration;
use crate::paths::Paths;

pub const NAME: &str = "booru";

/// User-Agent string sent with every API/download request. Most boorus block
/// the default `reqwest/<version>` UA, so this is non-optional.
const USER_AGENT: &str = concat!("wallrack/", env!("CARGO_PKG_VERSION"));
/// Fallback timeout for non-search HTTP (thumb cache, full-size download).
/// Search uses the configurable timeout from `BooruHttpPolicy` instead.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Hard cap on per-attempt backoff. The exponential doubles per retry; an
/// unhealthy site behind a 5xx loop would otherwise climb without bound.
const RETRY_BACKOFF_CAP: Duration = Duration::from_secs(30);

pub struct BooruIntegration;

impl Integration for BooruIntegration {
    fn name(&self) -> &'static str { NAME }
    fn label(&self) -> &'static str { "Booru" }

    // Booru entries are individual posts — drilling into a subfolder isn't
    // meaningful, and the existing drill UI would only confuse the picker.
    fn supports_drill(&self) -> bool { false }

    /// "Indexing" a booru means reading back whatever the last `booru search`
    /// wrote. This lets `wallrack index --integration=all` skip the booru
    /// without destroying its cached page.
    fn index(&self, paths: &Paths, _config: &Config) -> Result<Index> {
        paths.ensure_integration(NAME)?;
        let file = paths.index_file(NAME);
        if file.exists() {
            let raw = std::fs::read_to_string(&file)
                .with_context(|| format!("read booru index {}", file.display()))?;
            if !raw.trim().is_empty() {
                return Ok(serde_json::from_str(&raw)?);
            }
        }
        let empty = Index { integration: NAME.to_string(), entries: Vec::new() };
        crate::integrations::write_index(paths, &empty)?;
        Ok(empty)
    }

    /// Read the cached index and backfill missing thumb paths from disk —
    /// search calls before the "cache thumbs by default" fix stamped empty
    /// `thumb` fields even when the preview .png was on disk; recover those
    /// here so the picker shows icons without a re-search.
    fn read_index(&self, paths: &Paths) -> Result<Index> {
        let file = paths.index_file(NAME);
        if !file.exists() {
            return Err(anyhow!(
                "booru index not built — run `wallrack booru search` first"
            ));
        }
        let raw = std::fs::read_to_string(&file)?;
        let mut idx: Index = serde_json::from_str(&raw)?;
        let thumbs_dir = paths.thumbs_dir(NAME);
        for entry in &mut idx.entries {
            if entry.thumb.as_os_str().is_empty() {
                if let Some((site, post)) = entry.id.split_once(':') {
                    let guess = thumbs_dir.join(format!("{site}_{post}.png"));
                    if guess.is_file() {
                        entry.thumb = guess;
                    }
                }
            }
        }
        Ok(idx)
    }

    /// Download the entry's full-size image. The download URL lives in
    /// `Entry::workshop_id` — `BooruIntegration::save_search_as_index` puts
    /// it there so apply is a pure data lookup with no network round-trip
    /// other than the GET.
    ///
    /// Idempotent: if the predicted destination file already exists on disk,
    /// skip the GET. Booru filenames are content-addressable (md5 hashes for
    /// danbooru/gelbooru, `<id>.<ext>` for moebooru) so an existing file with
    /// the same name is the same image.
    fn apply(&self, entry: &Entry, _monitor: &str, _paths: &Paths, config: &Config) -> Result<()> {
        let url = entry
            .workshop_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!(
                "booru entry has no download url — re-run `wallrack booru search` to refresh the index"
            ))?;
        let into = config.booru.download_dir();
        std::fs::create_dir_all(&into)
            .with_context(|| format!("create download_dir {}", into.display()))?;
        let dest = into.join(filename_from_url(url, &entry.id));
        if dest.is_file() {
            log::info!("booru: already downloaded {} — skipping", dest.display());
            return Ok(());
        }
        download_to_file(url, &dest)?;
        log::info!("booru: downloaded {} -> {}", url, dest.display());
        Ok(())
    }

    fn watch_dirs(&self, _config: &Config) -> Vec<PathBuf> { Vec::new() }

    fn backend<'a>(&self, _config: &'a Config) -> &'a BackendConfig {
        // The booru integration has no [booru.backend] section — we never
        // run a shell apply_cmd (download is handled in-process). The trait
        // contract requires a &BackendConfig though, so route through a
        // static blank one.
        static EMPTY: std::sync::OnceLock<BackendConfig> = std::sync::OnceLock::new();
        EMPTY.get_or_init(BackendConfig::default)
    }

    fn default_backend(&self) -> BackendConfig {
        BackendConfig {
            // No real apply_cmd — the trait's apply() does the work directly.
            apply_cmd: None,
            // Single synthetic "download" entry so the monitor picker still
            // has something to render; the value is discarded by apply().
            monitors_cmd: Some("printf 'download\\n'".to_string()),
            current_image_cmd: None,
        }
    }
}

// ─── public API used by the CLI ────────────────────────────────────────────

/// One post returned by a booru search. Parsed from whichever JSON shape the
/// site uses; only the fields wallrack cares about are kept.
#[derive(Debug, Clone)]
pub struct BooruPost {
    pub id: u64,
    /// Site key (`konachan`, `danbooru`, …) — used to build a stable Entry id.
    pub site: String,
    pub tags: Vec<String>,
    /// "s" / "q" / "e" or the long-form danbooru variants. Empty when unknown.
    pub rating: String,
    pub file_url: String,
    pub preview_url: Option<String>,
    // width/height aren't surfaced in the Entry today but are cheap to keep
    // and lets a future "scale-aware" filter use them without re-parsing.
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
}

/// Hit the search endpoint for `site` with the given tags and page. `page`
/// is 1-based to match the moebooru/danbooru convention (gelbooru's 0-based
/// `pid` is translated internally).
///
/// Honors `policy.timeout` per attempt and retries up to `policy.max_retries`
/// times on transient failures (timeout, connect error, 5xx, 429). Bad-query
/// 4xx responses are returned immediately — retrying won't change them.
pub fn search(
    site_key: &str,
    site: &BooruSite,
    tags: &str,
    page: u32,
    limit: u32,
    policy: &BooruHttpPolicy,
) -> Result<Vec<BooruPost>> {
    let client = build_client_with_timeout(policy.timeout)?;
    let url = build_search_url(site, tags, page, limit);
    log::debug!("booru: GET {}", redact_credentials(&url));
    let body = http_get_with_retries(&client, &url, policy)?;
    parse_search(site_key, site.api_kind, &body)
        .with_context(|| format!("parse {} search response", site_key))
}

/// Persist the search results as the booru index so `wallrack list/view
/// --integration=booru` can render them. Returns the resulting [`Index`].
///
/// When `cache_thumbs` is set, this also downloads each post's preview into
/// `~/.cache/wallrack/booru/thumbs/` so picker formats have an `icon` to
/// render. Set to false on `--format=json` where the extra round-trips are
/// pure waste.
///
/// The `tags` / `page` arguments are used purely to refresh the canonical
/// picker state keys — the actual API hit happened in `search()`.
pub fn save_search_as_index(
    paths: &Paths,
    site_key: &str,
    tags: &str,
    page: u32,
    posts: &[BooruPost],
    cache_thumbs: bool,
    download_dir: &Path,
) -> Result<Index> {
    paths.ensure_integration(NAME)?;
    let thumbs_dir = paths.thumbs_dir(NAME);
    std::fs::create_dir_all(&thumbs_dir)
        .with_context(|| format!("create {}", thumbs_dir.display()))?;

    let client = if cache_thumbs { Some(build_client()?) } else { None };

    let entries: Vec<Entry> = posts
        .iter()
        .map(|p| post_to_entry(p, &thumbs_dir, download_dir, client.as_ref()))
        .collect();

    let idx = Index { integration: NAME.to_string(), entries };
    crate::integrations::write_index(paths, &idx)?;

    // Record the canonical search context in state so the picker can
    // paginate / re-site without the user retyping the query. The CLI and
    // the picker share these keys, so a CLI-driven search hands the picker
    // a working pagination context too. Best-effort; failures here aren't
    // fatal to the search itself.
    let _ = persist_last_search(paths, site_key, tags, page);

    Ok(idx)
}

/// Look up a post by its short numeric id within the cached booru index.
/// Returns the matching entry — the caller can hand it to
/// `BooruIntegration::apply` to actually download.
pub fn find_in_index(paths: &Paths, post_id: &str, site_key: Option<&str>) -> Result<Entry> {
    let file = paths.index_file(NAME);
    if !file.exists() {
        return Err(anyhow!(
            "no cached booru search — run `wallrack booru search` first"
        ));
    }
    let raw = std::fs::read_to_string(&file)?;
    let idx: Index = serde_json::from_str(&raw)?;
    let target_id_with_site: Option<String> =
        site_key.map(|s| format!("{s}:{post_id}"));
    idx.entries
        .iter()
        .find(|e| {
            if let Some(want) = target_id_with_site.as_deref() {
                e.id == want
            } else {
                e.id == post_id
                    || e.id
                        .rsplit_once(':')
                        .map(|(_, n)| n == post_id)
                        .unwrap_or(false)
            }
        })
        .cloned()
        .ok_or_else(|| anyhow!(
            "post id `{post_id}` not in cached search results — refine your search or pass a longer id"
        ))
}

// ─── HTTP helpers ──────────────────────────────────────────────────────────

fn build_client() -> Result<reqwest::blocking::Client> {
    build_client_with_timeout(HTTP_TIMEOUT)
}

fn build_client_with_timeout(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .context("build reqwest client")
}

/// Wrap a GET with exponential-backoff retries on transient failures.
/// Returns the first 4xx body immediately (other than 429); those are the
/// caller's fault — a malformed tag, bad auth — and won't fix themselves.
fn http_get_with_retries(
    client: &reqwest::blocking::Client,
    url: &str,
    policy: &BooruHttpPolicy,
) -> Result<String> {
    let mut backoff = policy.retry_backoff;
    let total_attempts = policy.max_retries.saturating_add(1);
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=total_attempts {
        match http_get_attempt(client, url) {
            Ok(body) => return Ok(body),
            Err(AttemptError::Permanent(err)) => return Err(err),
            Err(AttemptError::Transient(err)) => {
                if attempt == total_attempts {
                    last_err = Some(err);
                    break;
                }
                log::warn!(
                    "booru: attempt {}/{} failed ({:#}) — retrying in {:?}",
                    attempt, total_attempts, err, backoff
                );
                std::thread::sleep(backoff);
                // Exponential, capped. Avoids unbounded waits on a dead host.
                backoff = (backoff.saturating_mul(2)).min(RETRY_BACKOFF_CAP);
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("GET {url} failed after retries")))
}

/// Classify a single GET into permanent (don't retry) or transient (do retry).
/// A `reqwest::Error` from `send()` always counts as transient — those are
/// network-layer failures (connect refused, DNS, read timeout). HTTP status
/// errors split on 5xx + 429 (transient) vs everything else (permanent).
enum AttemptError {
    Transient(anyhow::Error),
    Permanent(anyhow::Error),
}

fn http_get_attempt(client: &reqwest::blocking::Client, url: &str) -> Result<String, AttemptError> {
    let resp = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            return Err(AttemptError::Transient(
                anyhow::Error::new(e).context(format!("GET {url}")),
            ));
        }
    };
    let status = resp.status();
    if status.is_success() {
        return resp
            .text()
            .with_context(|| format!("read body from {url}"))
            .map_err(AttemptError::Transient);
    }
    let body_preview = resp.text().unwrap_or_default();
    let err = anyhow!("GET {url} returned HTTP {status}: {body_preview}");
    if status.is_server_error() || status.as_u16() == 429 {
        Err(AttemptError::Transient(err))
    } else {
        Err(AttemptError::Permanent(err))
    }
}

/// Stream a URL to disk. Used for both full-size downloads and preview thumbs.
fn download_to_file(url: &str, dest: &Path) -> Result<u64> {
    let client = build_client()?;
    let mut resp = client.get(url).send().with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("GET {url} returned HTTP {status}"));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = dest.with_extension(format!(
        "{}.part",
        dest.extension().and_then(|s| s.to_str()).unwrap_or("dl")
    ));
    let mut file = std::fs::File::create(&tmp)
        .with_context(|| format!("create {}", tmp.display()))?;
    let bytes = std::io::copy(&mut resp, &mut file)
        .with_context(|| format!("stream body to {}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    Ok(bytes)
}

// ─── URL building & parsing ────────────────────────────────────────────────

fn build_search_url(site: &BooruSite, tags: &str, page: u32, limit: u32) -> String {
    let base = site.base_url.trim_end_matches('/');
    let tags_enc = encode_query(tags);
    let auth = auth_query(site);
    match site.api_kind {
        BooruApiKind::Moebooru => format!(
            "{base}/post.json?tags={tags_enc}&page={page}&limit={limit}{auth}"
        ),
        BooruApiKind::Danbooru => format!(
            "{base}/posts.json?tags={tags_enc}&page={page}&limit={limit}{auth}"
        ),
        BooruApiKind::Gelbooru => {
            // Gelbooru pages are 0-based via `pid`.
            let pid = page.saturating_sub(1);
            format!(
                "{base}/index.php?page=dapi&s=post&q=index&json=1&tags={tags_enc}&pid={pid}&limit={limit}{auth}"
            )
        }
    }
}

/// Build the per-site auth query suffix. Each booru family wires this
/// differently: moebooru wants `login` + `password_hash`, danbooru wants
/// `login` + `api_key`, gelbooru wants `user_id` + `api_key`. Missing
/// credentials produce an empty suffix (anonymous request).
fn auth_query(site: &BooruSite) -> String {
    match site.api_kind {
        BooruApiKind::Moebooru => moebooru_auth(site),
        BooruApiKind::Danbooru => match (&site.login, &site.api_key) {
            (Some(login), Some(key)) => format!(
                "&login={}&api_key={}",
                encode_query(login),
                encode_query(key)
            ),
            _ => String::new(),
        },
        BooruApiKind::Gelbooru => match (&site.user_id, &site.api_key) {
            (Some(uid), Some(key)) => format!(
                "&user_id={}&api_key={}",
                encode_query(uid),
                encode_query(key)
            ),
            _ => String::new(),
        },
    }
}

/// Moebooru wants `login=user&password_hash=hex_sha1`. The hash is
/// `SHA1(salt_template_with_{}_replaced_by_password)`. Each site has its own
/// salt (konachan ≠ yande.re), shipped as a builtin. Users can either
/// stash the plaintext `password` (we hash it) or paste the
/// `password_hash` directly from their browser cookies.
fn moebooru_auth(site: &BooruSite) -> String {
    use sha1::{Digest, Sha1};
    let Some(login) = site.login.as_deref().filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let hash = if let Some(h) = site.password_hash.as_deref().filter(|s| !s.is_empty()) {
        h.to_string()
    } else if let (Some(pw), Some(salt)) = (
        site.password.as_deref().filter(|s| !s.is_empty()),
        site.password_salt.as_deref().filter(|s| !s.is_empty()),
    ) {
        let salted = salt.replacen("{}", pw, 1);
        hex::encode(Sha1::digest(salted.as_bytes()))
    } else {
        log::warn!(
            "moebooru auth: login set but no password/password_hash (or salt missing) — \
             sending anonymous request"
        );
        return String::new();
    };
    format!(
        "&login={}&password_hash={}",
        encode_query(login),
        encode_query(&hash)
    )
}

/// Mask credential-bearing query params so URLs are safe to put in logs and
/// notifications. The booru never returns these values, so masking is
/// one-way — we don't have to round-trip them.
fn redact_credentials(url: &str) -> String {
    const SENSITIVE: &[&str] = &["api_key", "password_hash", "password", "login", "user_id"];
    let Some((base, query)) = url.split_once('?') else { return url.to_string() };
    let parts = query.split('&').map(|kv| match kv.split_once('=') {
        Some((k, _)) if SENSITIVE.contains(&k) => format!("{k}=***"),
        _ => kv.to_string(),
    });
    let masked: Vec<String> = parts.collect();
    format!("{base}?{}", masked.join("&"))
}

/// Bare-bones percent encoder. Booru tag queries are whitespace-separated
/// `a-z0-9_:()-` plus the occasional `*`/`!`/`%`, none of which is reserved
/// in a query string — encoding spaces and a handful of meta chars is enough.
fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push('+'),
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

// ─── response parsers ─────────────────────────────────────────────────────

fn parse_search(site: &str, kind: BooruApiKind, body: &str) -> Result<Vec<BooruPost>> {
    match kind {
        BooruApiKind::Moebooru => parse_moebooru(site, body),
        BooruApiKind::Danbooru => parse_danbooru(site, body),
        BooruApiKind::Gelbooru => parse_gelbooru(site, body),
    }
}

#[derive(Debug, Deserialize)]
struct MoebooruPost {
    id: u64,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    file_url: String,
    #[serde(default)]
    sample_url: Option<String>,
    #[serde(default)]
    preview_url: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

fn parse_moebooru(site: &str, body: &str) -> Result<Vec<BooruPost>> {
    let raw: Vec<MoebooruPost> = serde_json::from_str(body)?;
    Ok(raw
        .into_iter()
        .filter(|p| !p.file_url.is_empty())
        .map(|p| BooruPost {
            id: p.id,
            site: site.to_string(),
            tags: split_tags(&p.tags),
            rating: p.rating,
            file_url: ensure_scheme(p.file_url),
            preview_url: p
                .preview_url
                .or(p.sample_url)
                .map(ensure_scheme),
            width: p.width,
            height: p.height,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct DanbooruPost {
    id: u64,
    #[serde(default)]
    tag_string: String,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    file_url: Option<String>,
    #[serde(default)]
    large_file_url: Option<String>,
    #[serde(default)]
    preview_file_url: Option<String>,
    #[serde(default)]
    image_width: u32,
    #[serde(default)]
    image_height: u32,
}

fn parse_danbooru(site: &str, body: &str) -> Result<Vec<BooruPost>> {
    // Danbooru sometimes responds `{"success": false, ...}` instead of an
    // array when a tag query is invalid; bubble that up rather than letting
    // serde error on the wrong shape.
    let trimmed = body.trim_start();
    if trimmed.starts_with('{') {
        return Err(anyhow!("danbooru returned an error object: {}", trimmed));
    }
    let raw: Vec<DanbooruPost> = serde_json::from_str(body)?;
    Ok(raw
        .into_iter()
        .filter_map(|p| {
            let url = p.file_url.clone().or_else(|| p.large_file_url.clone())?;
            Some(BooruPost {
                id: p.id,
                site: site.to_string(),
                tags: split_tags(&p.tag_string),
                rating: p.rating,
                file_url: url,
                preview_url: p.preview_file_url,
                width: p.image_width,
                height: p.image_height,
            })
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct GelbooruEnvelope {
    #[serde(default)]
    post: Vec<GelbooruPost>,
}

#[derive(Debug, Deserialize)]
struct GelbooruPost {
    id: u64,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    rating: String,
    #[serde(default)]
    file_url: String,
    #[serde(default)]
    preview_url: Option<String>,
    #[serde(default)]
    sample_url: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

fn parse_gelbooru(site: &str, body: &str) -> Result<Vec<BooruPost>> {
    // Gelbooru: newer responses are `{"@attributes":..,"post":[...]}`; older
    // safebooru returns just the bare array. Try both.
    let trimmed = body.trim_start();
    let raw: Vec<GelbooruPost> = if trimmed.starts_with('[') {
        serde_json::from_str(body)?
    } else if trimmed.is_empty() {
        Vec::new()
    } else {
        let env: GelbooruEnvelope = serde_json::from_str(body)?;
        env.post
    };
    Ok(raw
        .into_iter()
        .filter(|p| !p.file_url.is_empty())
        .map(|p| BooruPost {
            id: p.id,
            site: site.to_string(),
            tags: split_tags(&p.tags),
            rating: p.rating,
            file_url: ensure_scheme(p.file_url),
            preview_url: p
                .sample_url
                .filter(|s| !s.is_empty())
                .or(p.preview_url)
                .map(ensure_scheme),
            width: p.width,
            height: p.height,
        })
        .collect())
}

fn split_tags(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

/// Some boorus return scheme-relative URLs (`//foo/bar.jpg`). Default to
/// https since every modern booru speaks it.
fn ensure_scheme(url: String) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url
    }
}

// ─── post → Entry conversion ──────────────────────────────────────────────

fn post_to_entry(
    p: &BooruPost,
    thumbs_dir: &Path,
    download_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
) -> Entry {
    let filename = filename_from_url(&p.file_url, &format!("{}:{}", p.site, p.id));
    let predicted = download_dir.join(&filename);

    let thumb_path = client
        .and_then(|c| {
            let url = p.preview_url.as_deref()?;
            let dest = thumbs_dir.join(format!("{}_{}.png", p.site, p.id));
            if !dest.exists() {
                if let Err(err) = cache_preview(c, url, &dest) {
                    log::debug!("booru: preview cache failed for {url}: {err:#}");
                    return None;
                }
            }
            Some(dest)
        })
        .unwrap_or_default();

    let title = if p.tags.is_empty() {
        format!("{} #{}", p.site, p.id)
    } else {
        // Cap the title at the first few tags — long booru tag lists make
        // every row in the picker the same width.
        let head: Vec<&str> = p.tags.iter().take(4).map(|s| s.as_str()).collect();
        format!("{} #{} — {}", p.site, p.id, head.join(" "))
    };

    Entry {
        integration: NAME.to_string(),
        id: format!("{}:{}", p.site, p.id),
        title,
        source: predicted,
        thumb: thumb_path,
        rating: map_rating(&p.rating),
        tags: p.tags.clone(),
        // workshop_id repurposed as the download URL — apply() reads it.
        workshop_id: Some(p.file_url.clone()),
        // subfolder repurposed as the site, so multi-site mixed searches
        // could be grouped if we ever want it.
        subfolder: p.site.clone(),
    }
}

/// Map booru rating short-codes onto wallrack's existing `Rating` strings so
/// the standard rating filter (`Mature`/`Questionable`/`Everyone`) works.
fn map_rating(s: &str) -> String {
    match s {
        "s" | "safe" | "g" | "general" => "Everyone".into(),
        "q" | "questionable" => "Questionable".into(),
        "e" | "explicit" => "Mature".into(),
        _ => String::new(),
    }
}

/// Pre-cache a preview thumb on disk. We write whatever bytes the booru
/// returned (jpg/png) unchanged — the picker formats render them directly.
fn cache_preview(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
) -> Result<()> {
    let mut resp = client.get(url).send().with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("preview GET {url} returned HTTP {status}"));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = Vec::new();
    resp.read_to_end(&mut buf)?;
    std::fs::write(dest, &buf)?;
    Ok(())
}

fn filename_from_url(url: &str, fallback_stem: &str) -> String {
    // Strip query/fragment first, then take the last path segment.
    let stripped = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('#')
        .next()
        .unwrap_or(url);
    let name = stripped.rsplit('/').next().unwrap_or("");
    if name.is_empty() || !name.contains('.') {
        // Fallback: <site:id>.bin — at least it lands somewhere reasonable.
        let safe = fallback_stem.replace([':', '/'], "_");
        format!("{safe}.bin")
    } else {
        name.to_string()
    }
}

// ─── last-search state ────────────────────────────────────────────────────

fn persist_last_search(paths: &Paths, site_key: &str, tags: &str, page: u32) -> Result<()> {
    use crate::state::{State, keys};
    let state_path = paths.state_file();
    let mut state = State::load(&state_path)?;
    state.set(keys::BOORU_SITE, site_key);
    state.set(keys::BOORU_QUERY, tags);
    state.set(keys::BOORU_PAGE, page.to_string());
    state.save(&state_path)?;
    Ok(())
}

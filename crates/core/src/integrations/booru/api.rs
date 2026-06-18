//! Booru search API — HTTP client, retry policy, URL building.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::config::{BooruApiKind, BooruHttpPolicy, BooruSite};

use super::auth::{auth_query, encode_query, redact_credentials};
use super::parse::{BooruPost, parse_search};

/// User-Agent string sent with every API/download request. Most boorus block
/// the default `reqwest/<version>` UA, so this is non-optional.
const USER_AGENT: &str = concat!("wallrack/", env!("CARGO_PKG_VERSION"));
/// Fallback timeout for non-search HTTP (thumb cache, full-size download).
/// Search uses the configurable timeout from `BooruHttpPolicy` instead.
pub(super) const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Hard cap on per-attempt backoff. The exponential doubles per retry; an
/// unhealthy site behind a 5xx loop would otherwise climb without bound.
const RETRY_BACKOFF_CAP: Duration = Duration::from_secs(30);

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

pub(super) fn build_client() -> Result<reqwest::blocking::Client> {
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

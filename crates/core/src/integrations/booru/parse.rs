//! Booru search response parsers — one per API family. The three families
//! return enough divergent field names that an enum dispatch is cleaner
//! than serde aliases.

use anyhow::{Result, anyhow};
use serde::Deserialize;

use crate::config::BooruApiKind;

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

pub(super) fn parse_search(site: &str, kind: BooruApiKind, body: &str) -> Result<Vec<BooruPost>> {
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
            preview_url: p.preview_url.or(p.sample_url).map(ensure_scheme),
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

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moebooru_parses_minimal_post() {
        let body = r#"[
            {
                "id": 42,
                "tags": "scenery sky blue",
                "rating": "s",
                "file_url": "https://konachan.com/image/abc.jpg",
                "sample_url": "https://konachan.com/sample/abc.jpg",
                "preview_url": "https://konachan.com/preview/abc.jpg",
                "width": 1920,
                "height": 1080
            }
        ]"#;
        let posts = parse_search("konachan", BooruApiKind::Moebooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        let p = &posts[0];
        assert_eq!(p.id, 42);
        assert_eq!(p.site, "konachan");
        assert_eq!(p.tags, vec!["scenery", "sky", "blue"]);
        assert_eq!(p.rating, "s");
        assert_eq!(p.file_url, "https://konachan.com/image/abc.jpg");
        assert_eq!(
            p.preview_url.as_deref(),
            Some("https://konachan.com/preview/abc.jpg")
        );
        assert_eq!(p.width, 1920);
        assert_eq!(p.height, 1080);
    }

    #[test]
    fn moebooru_filters_posts_with_empty_file_url() {
        let body = r#"[
            {"id": 1, "tags": "a", "file_url": ""},
            {"id": 2, "tags": "b", "file_url": "https://example.com/x.jpg"}
        ]"#;
        let posts = parse_search("konachan", BooruApiKind::Moebooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, 2);
    }

    #[test]
    fn moebooru_falls_back_to_sample_url_when_no_preview() {
        let body = r#"[
            {
                "id": 1,
                "tags": "x",
                "file_url": "https://example.com/x.jpg",
                "sample_url": "https://example.com/sample.jpg"
            }
        ]"#;
        let posts = parse_search("yandere", BooruApiKind::Moebooru, body).unwrap();
        assert_eq!(
            posts[0].preview_url.as_deref(),
            Some("https://example.com/sample.jpg")
        );
    }

    #[test]
    fn moebooru_promotes_scheme_relative_urls_to_https() {
        let body = r#"[
            {
                "id": 1,
                "tags": "x",
                "file_url": "//cdn.example.com/x.jpg",
                "preview_url": "//cdn.example.com/p.jpg"
            }
        ]"#;
        let posts = parse_search("konachan", BooruApiKind::Moebooru, body).unwrap();
        assert_eq!(posts[0].file_url, "https://cdn.example.com/x.jpg");
        assert_eq!(
            posts[0].preview_url.as_deref(),
            Some("https://cdn.example.com/p.jpg")
        );
    }

    #[test]
    fn danbooru_parses_tag_string_field() {
        let body = r#"[
            {
                "id": 7,
                "tag_string": "red blue green",
                "rating": "g",
                "file_url": "https://cdn.donmai.us/full.jpg",
                "preview_file_url": "https://cdn.donmai.us/preview.jpg",
                "image_width": 800,
                "image_height": 600
            }
        ]"#;
        let posts = parse_search("danbooru", BooruApiKind::Danbooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].tags, vec!["red", "blue", "green"]);
        assert_eq!(posts[0].file_url, "https://cdn.donmai.us/full.jpg");
        assert_eq!(posts[0].width, 800);
        assert_eq!(posts[0].height, 600);
    }

    #[test]
    fn danbooru_falls_back_to_large_file_url_when_full_missing() {
        let body = r#"[
            {
                "id": 8,
                "tag_string": "x",
                "large_file_url": "https://cdn.donmai.us/large.jpg"
            }
        ]"#;
        let posts = parse_search("danbooru", BooruApiKind::Danbooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].file_url, "https://cdn.donmai.us/large.jpg");
    }

    #[test]
    fn danbooru_drops_posts_without_any_image_url() {
        let body = r#"[
            {"id": 1, "tag_string": "x"},
            {"id": 2, "tag_string": "y", "file_url": "https://e.com/y.jpg"}
        ]"#;
        let posts = parse_search("danbooru", BooruApiKind::Danbooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, 2);
    }

    #[test]
    fn danbooru_error_object_returns_err_instead_of_empty() {
        let body = r#"{"success": false, "message": "bad tag"}"#;
        let err = parse_search("danbooru", BooruApiKind::Danbooru, body).unwrap_err();
        assert!(
            err.to_string()
                .contains("danbooru returned an error object")
        );
    }

    #[test]
    fn gelbooru_parses_envelope_with_post_array() {
        let body = r#"{
            "@attributes": {"count": 1},
            "post": [
                {
                    "id": 99,
                    "tags": "alpha beta",
                    "rating": "general",
                    "file_url": "https://gelbooru.com/full.jpg",
                    "sample_url": "https://gelbooru.com/sample.jpg",
                    "preview_url": "https://gelbooru.com/preview.jpg",
                    "width": 100,
                    "height": 200
                }
            ]
        }"#;
        let posts = parse_search("gelbooru", BooruApiKind::Gelbooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, 99);
        assert_eq!(posts[0].tags, vec!["alpha", "beta"]);
        // Sample URL preferred over preview when present and non-empty.
        assert_eq!(
            posts[0].preview_url.as_deref(),
            Some("https://gelbooru.com/sample.jpg")
        );
    }

    #[test]
    fn gelbooru_parses_bare_array_for_safebooru_shape() {
        let body = r#"[
            {
                "id": 5,
                "tags": "x",
                "file_url": "https://safebooru.org/x.jpg"
            }
        ]"#;
        let posts = parse_search("safebooru", BooruApiKind::Gelbooru, body).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, 5);
    }

    #[test]
    fn gelbooru_treats_empty_body_as_no_posts() {
        let posts = parse_search("gelbooru", BooruApiKind::Gelbooru, "").unwrap();
        assert!(posts.is_empty());
    }

    #[test]
    fn gelbooru_falls_back_to_preview_when_sample_empty() {
        let body = r#"{
            "post": [
                {
                    "id": 1,
                    "tags": "x",
                    "file_url": "https://gelbooru.com/full.jpg",
                    "sample_url": "",
                    "preview_url": "https://gelbooru.com/preview.jpg"
                }
            ]
        }"#;
        let posts = parse_search("gelbooru", BooruApiKind::Gelbooru, body).unwrap();
        assert_eq!(
            posts[0].preview_url.as_deref(),
            Some("https://gelbooru.com/preview.jpg")
        );
    }
}

//! Booru post → on-disk artifacts: index file, preview thumbs, full-size
//! downloads, and the last-search picker state.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::entry::{Entry, Index};
use crate::paths::Paths;

use super::NAME;
use super::api::build_client;
use super::parse::BooruPost;

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

    let idx = Index {
        integration: NAME.to_string(),
        entries,
    };
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
/// Returns the matching entry — the caller can hand it to `BooruIntegration::apply`
/// to actually download.
pub fn find_in_index(paths: &Paths, post_id: &str, site_key: Option<&str>) -> Result<Entry> {
    let file = paths.index_file(NAME);
    if !file.exists() {
        return Err(anyhow!(
            "no cached booru search — run `wallrack booru search` first"
        ));
    }
    let raw = std::fs::read_to_string(&file)?;
    let idx: Index = serde_json::from_str(&raw)?;
    let target_id_with_site: Option<String> = site_key.map(|s| format!("{s}:{post_id}"));
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
        .ok_or_else(|| {
            anyhow!(
                "post id `{post_id}` not in cached search results — refine your search or pass a longer id"
            )
        })
}

/// Stream a URL to disk. Used for both full-size downloads and preview thumbs.
pub(super) fn download_to_file(url: &str, dest: &Path) -> Result<u64> {
    let client = build_client()?;
    let mut resp = client
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?;
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
fn cache_preview(client: &reqwest::blocking::Client, url: &str, dest: &Path) -> Result<()> {
    let mut resp = client
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?;
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

pub(super) fn filename_from_url(url: &str, fallback_stem: &str) -> String {
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


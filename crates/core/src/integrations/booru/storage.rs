//! Booru post → on-disk artifacts: index file, preview thumbs, full-size
//! downloads, and the last-search picker state.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

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
            let id = e.id();
            if let Some(want) = target_id_with_site.as_deref() {
                id == want
            } else {
                id == post_id
                    || id.rsplit_once(':')
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

/// Stream a URL to disk. Used for full-size booru downloads — reads the body
/// in chunks so a [`DownloadProgress`] reporter can fire spinner notifications
/// with bytes-downloaded / remaining while the GET is in flight.
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

    let total = resp.content_length();
    let label = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    let mut progress = DownloadProgress::new(label, total);
    progress.start();

    let mut downloaded: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = resp
            .read(&mut buf)
            .with_context(|| format!("read body from {url}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .with_context(|| format!("write to {}", tmp.display()))?;
        downloaded += n as u64;
        progress.tick(downloaded);
    }
    drop(file);
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    progress.finish(downloaded);
    Ok(downloaded)
}

/// `notify-send`-driven progress reporter for booru downloads. A single
/// notification is replaced in place (`--replace-id`) so the spinner +
/// MB-downloaded / remaining counter live-updates rather than spamming.
struct DownloadProgress {
    label: String,
    total: Option<u64>,
    start: Instant,
    last_notif: Instant,
    notif_id: Option<String>,
    rendered: bool,
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const NOTIFY_THROTTLE_MS: u128 = 500;
/// Booru filenames (esp. danbooru md5 hashes) blow out the notification width.
const LABEL_MAX_CHARS: usize = 25;

impl DownloadProgress {
    fn new(label: String, total: Option<u64>) -> Self {
        let initial_notif_id = std::env::var("WALLRACK_NOTIF_ID")
            .ok()
            .filter(|s| !s.is_empty());
        Self {
            label: truncate_label(&label, LABEL_MAX_CHARS),
            total,
            start: Instant::now(),
            last_notif: Instant::now(),
            notif_id: initial_notif_id,
            rendered: false,
        }
    }

    fn start(&mut self) {
        self.notify(0, false);
        self.rendered = true;
        self.last_notif = Instant::now();
    }

    fn tick(&mut self, downloaded: u64) {
        if self.rendered && self.last_notif.elapsed().as_millis() < NOTIFY_THROTTLE_MS {
            return;
        }
        self.last_notif = Instant::now();
        self.rendered = true;
        self.notify(downloaded, false);
    }

    fn finish(&mut self, downloaded: u64) {
        self.notify(downloaded, true);
    }

    fn notify(&mut self, downloaded: u64, done: bool) {
        let spin = SPINNER[(self.start.elapsed().as_millis() / 80) as usize % SPINNER.len()];
        let body = if done {
            format!("✓ {} — {} downloaded", self.label, format_bytes(downloaded))
        } else if let Some(total) = self.total {
            let remaining = total.saturating_sub(downloaded);
            let pct = downloaded
                .saturating_mul(100)
                .checked_div(total)
                .map(|p| p.min(100))
                .unwrap_or(100);
            format!(
                "{spin} {} — {} / {} ({} left, {pct}%)",
                self.label,
                format_bytes(downloaded),
                format_bytes(total),
                format_bytes(remaining),
            )
        } else {
            format!("{spin} {} — {}", self.label, format_bytes(downloaded))
        };
        let expire_ms = if done { "3000" } else { "0" };

        let mut cmd = Command::new("notify-send");
        cmd.arg("--print-id")
            .arg(format!("--expire-time={expire_ms}"))
            .arg("Wallrack booru")
            .arg(&body);
        if let Some(ref id) = self.notif_id {
            cmd.arg(format!("--replace-id={id}"));
        }
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !id_str.is_empty() {
                    self.notif_id = Some(id_str);
                }
            }
        }
    }
}

/// Cap a label at `max` chars (not bytes — respect multi-byte filenames),
/// substituting a single-char ellipsis when the original overflows.
fn truncate_label(label: &str, max: usize) -> String {
    if label.chars().count() <= max {
        return label.to_string();
    }
    let head: String = label.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{:.2} MB", mb)
    } else {
        let kb = bytes as f64 / 1024.0;
        format!("{:.0} KB", kb)
    }
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

    Entry::BooruPost {
        id: format!("{}:{}", p.site, p.id),
        site: p.site.clone(),
        post_id: p.id,
        title,
        thumb: thumb_path,
        tags: p.tags.clone(),
        rating: map_rating(&p.rating),
        download_url: p.file_url.clone(),
        predicted_path: predicted,
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
    let state = State::open(paths.store())?;
    state.set(keys::BOORU_SITE, site_key)?;
    state.set(keys::BOORU_QUERY, tags)?;
    state.set(keys::BOORU_PAGE, page.to_string())?;
    Ok(())
}


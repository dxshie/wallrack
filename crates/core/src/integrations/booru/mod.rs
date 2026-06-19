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

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use crate::config::{BackendConfig, Config};
use crate::entry::{Entry, Index};
use crate::integrations::Integration;
use crate::paths::Paths;

mod api;
mod auth;
mod parse;
mod storage;

pub use api::search;
pub use parse::BooruPost;
pub use storage::{find_in_index, save_search_as_index};

pub const NAME: &str = "booru";

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
                match serde_json::from_str(&raw) {
                    Ok(idx) => return Ok(idx),
                    Err(err) => {
                        // Most likely a pre-0.3 index in the legacy flat shape.
                        // Booru indexes are rebuilt by any `booru search`, so
                        // clobber the file rather than fail the whole index run.
                        log::warn!(
                            "booru: stale cached search ({}) — clearing; run `wallrack booru search` to rebuild",
                            err
                        );
                    }
                }
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
            if entry.thumb().as_os_str().is_empty() {
                if let Some((site, post)) = entry.id().split_once(':') {
                    let guess = thumbs_dir.join(format!("{site}_{post}.png"));
                    if guess.is_file() {
                        entry.set_thumb(guess);
                    }
                }
            }
        }
        Ok(idx)
    }

    /// Download the entry's full-size image. The download URL lives in
    /// `Entry::workshop_id` — `save_search_as_index` puts it there so apply
    /// is a pure data lookup with no network round-trip other than the GET.
    ///
    /// Idempotent: if the predicted destination file already exists on disk,
    /// skip the GET. Booru filenames are content-addressable (md5 hashes for
    /// danbooru/gelbooru, `<id>.<ext>` for moebooru) so an existing file with
    /// the same name is the same image.
    fn apply(&self, entry: &Entry, _monitor: &str, _paths: &Paths, config: &Config) -> Result<()> {
        let url = entry
            .download_url()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "booru entry has no download url — re-run `wallrack booru search` to refresh the index"
                )
            })?;
        let into = config.booru.download_dir();
        std::fs::create_dir_all(&into)
            .with_context(|| format!("create download_dir {}", into.display()))?;
        let dest = into.join(storage::filename_from_url(url, entry.id()));
        if dest.is_file() {
            log::info!("booru: already downloaded {} — skipping", dest.display());
            return Ok(());
        }
        storage::download_to_file(url, &dest)?;
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

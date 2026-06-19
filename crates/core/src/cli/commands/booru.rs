use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};

use crate::config::Config;
use crate::entry::Entry;
use crate::favorites::Favorites;
use crate::integrations::{self, booru};
use crate::output::{Action, Format, Row, ViewHints, write_rows};
use crate::paths::Paths;
use crate::state::State;

use super::super::args::BooruCmd;

pub(in crate::cli) fn run(paths: &Paths, config: &Config, cmd: BooruCmd) -> Result<ExitCode> {
    match cmd {
        BooruCmd::Search {
            site,
            tags,
            page,
            limit,
            format,
            no_thumbs,
        } => run_search(paths, config, site, tags, page, limit, format, no_thumbs),
        BooruCmd::Download { site, id } => run_download(paths, config, site, id),
        BooruCmd::CurrentSite => run_current_site(paths, config),
        BooruCmd::Sites { format } => run_sites(config, format),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_search(
    paths: &Paths,
    config: &Config,
    site: Option<String>,
    tags: String,
    page: u32,
    limit: Option<u32>,
    format: Format,
    no_thumbs: bool,
) -> Result<ExitCode> {
    let site_key = site
        .or_else(|| Some(config.booru.default_site.clone()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!("no site specified — pass --site or set `booru.default_site` in config")
        })?;
    let site_def = config.booru.resolve_site(&site_key).ok_or_else(|| {
        anyhow!(
            "unknown booru site `{site_key}` — known: {}",
            config
                .booru
                .resolved_sites()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let limit = limit.unwrap_or(config.booru.per_page).clamp(1, 200);
    let policy = config.booru.http_policy();
    let posts = booru::search(&site_key, &site_def, &tags, page, limit, &policy)
        .with_context(|| format!("search {site_key}"))?;
    log::info!(
        "booru: {} posts from {site_key} (page {page}, query `{tags}`)",
        posts.len()
    );
    // Default to caching preview thumbs — the picker drives a JSON
    // round-trip (it discards stdout and re-renders via `wallrack view`)
    // yet still needs `icon` paths populated. Opt out with --no-thumbs
    // when scripting.
    let want_thumbs = !no_thumbs;
    let download_dir = config.booru.download_dir();
    let idx = booru::save_search_as_index(
        paths,
        &site_key,
        &tags,
        page,
        &posts,
        want_thumbs,
        &download_dir,
    )?;

    let favorites = Favorites::open(paths.store())?;
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    let entries: Vec<&Entry> = idx.entries.iter().collect();
    let rows: Vec<Row<'_>> = entries
        .iter()
        .map(|e| Row::Entry {
            entry: e,
            favorite: favorites.is_favorite(e.integration(), e.id()),
            label: None,
            action: Some(Action::BooruSearchHit {
                id: e.id().to_string(),
            }),
        })
        .collect();
    let hints = ViewHints {
        prompt: format!("booru/{site_key} p{page}"),
        message: format!(
            "{} results — `wallrack booru download <id>` to save",
            entries.len()
        ),
        ..ViewHints::default()
    };
    write_rows(&mut out, &rows, &hints, format)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

fn run_download(
    paths: &Paths,
    config: &Config,
    site: Option<String>,
    id: String,
) -> Result<ExitCode> {
    // Accept either the bare numeric id or the full `site:id` slug.
    let (site_hint, post_id) = match id.split_once(':') {
        Some((s, n)) => (Some(s.to_string()), n.to_string()),
        None => (site.clone(), id.clone()),
    };
    let site_hint = site.clone().or(site_hint);
    let entry = booru::find_in_index(paths, &post_id, site_hint.as_deref())?;
    let integ = integrations::by_name("booru")?;
    integ.apply(&entry, "", paths, config)?;
    // entry.source is the predicted destination path that apply() wrote to —
    // print it verbatim so the rofi wrapper can capture
    // `$(wallrack booru download <id>)` and feed it into the wallpaper monitor
    // picker.
    println!("{}", entry.source().display());
    Ok(ExitCode::SUCCESS)
}

fn run_current_site(paths: &Paths, config: &Config) -> Result<ExitCode> {
    let st = State::open(paths.store())?;
    let resolved = if let Some(s) = st.booru_site() {
        s.to_string()
    } else if !config.booru.default_site.is_empty() {
        config.booru.default_site.clone()
    } else {
        config
            .booru
            .resolved_sites()
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "konachan".to_string())
    };
    println!("{resolved}");
    Ok(ExitCode::SUCCESS)
}

fn run_sites(config: &Config, format: Format) -> Result<ExitCode> {
    let sites = config.booru.resolved_sites();
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    match format {
        Format::Json => {
            serde_json::to_writer(&mut out, &sites)?;
        }
        Format::Rofi | Format::Walker | Format::Wofi | Format::Fuzzel => {
            for (k, v) in &sites {
                writeln!(out, "{k}\t{}", v.base_url)?;
            }
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

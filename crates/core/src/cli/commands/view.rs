//! `wallrack view` — render the picker view based on persisted state.
//! Dispatches to one of several sub-views (booru, tag-editor, add-tag,
//! tag-mode select, or the default list/drill/grouped view).

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::Result;

use crate::config::Config;
use crate::favorites::Favorites;
use crate::integrations;
use crate::output::{Format, Row, ViewHints, write_rows};
use crate::paths::Paths;
use crate::state::{self, State};

use super::super::render::{
    emit_drill_view, emit_empty_view, emit_flat, emit_grouped_view, filter_entries,
};
use super::tags;

pub(in crate::cli) fn run(paths: &Paths, format: Format) -> Result<ExitCode> {
    let state = State::load(&paths.state_file())?;
    let integration = state
        .get_or(state::keys::PICKER_MODE, "wallpaper")
        .to_string();
    let view_mode = state.get_or(state::keys::VIEW_MODE, "all").to_string();
    let drill = state.get_or(state::keys::DRILL_PATH, "").to_string();
    let tag_filter = state.get_or(state::keys::TAG_FILTER, "").to_string();
    let rating = state.get_or(state::keys::RATING, "").to_string();
    let tag_mode = state.get_or(state::keys::TAG_MODE, "").to_string();
    let tag_edit_target = state.get_or(state::keys::TAG_EDIT_TARGET, "").to_string();
    let tag_add_mode = state.get_or(state::keys::TAG_ADD_MODE, "").to_string();

    // The tag-editor sub-views are state-driven so wrappers don't have to
    // know how to render them. Order matches the rofi script's dispatch:
    // add-mode wins over edit-target, which wins over tag-filter selection.
    if tag_add_mode == "on" {
        return cmd_add_tag_view(paths, &integration, &tag_edit_target, format);
    }
    if !tag_edit_target.is_empty() {
        return cmd_tag_editor_view(paths, &integration, &tag_edit_target, format);
    }

    // Tag selection view short-circuits everything else.
    if tag_mode == "selecting" {
        return tags::run(paths, Some(&integration), format);
    }

    // The booru integration is search-driven — render its own header rows
    // (search prompt, site, pagination) before any entries. Bypasses the
    // favorites/tag/drill machinery that doesn't apply here.
    if integration == "booru" {
        return cmd_booru_view(paths, &state, format);
    }

    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::load(&paths.favorites_file())?;

    let favorites_only = view_mode == "favorites";
    let tag = (!tag_filter.is_empty()).then_some(tag_filter.as_str());
    let rating_opt = (!rating.is_empty() && rating != "All").then_some(rating.as_str());
    let folder_opt = (!drill.is_empty()).then_some(drill.as_str());

    let filtered = filter_entries(
        &index,
        &favorites,
        favorites_only,
        tag,
        rating_opt,
        folder_opt,
    );

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    // Empty top-level view → render a placeholder row so rofi doesn't exit.
    // The drill view always carries a "← Back" row so it can stand on its own.
    if filtered.is_empty() && folder_opt.is_none() {
        emit_empty_view(&mut out, &integration, favorites_only, tag, format)?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(folder_path) = folder_opt {
        emit_drill_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            folder_path,
            favorites_only,
            tag,
            format,
        )?;
    } else if drill.is_empty() && integ.supports_drill() && !favorites_only {
        // Grouping collapses subfolders into folder rows, which is wrong for
        // the favorites view: a favorite is an individual image and Alt+3 on
        // a folder row can't recover the real entry id.
        emit_grouped_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag,
            format,
        )?;
    } else {
        emit_flat(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag,
            format,
        )?;
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Render the booru picker view. Two sub-views:
///   * `BOORU_SEARCH_MODE = on` → search prompt (single Cancel row, picker
///     uses rofi's allow-custom to capture the typed query).
///   * normal → control rows (Search / Site / Prev / Next) followed by the
///     cached posts from the last `wallrack booru search`.
fn cmd_booru_view(paths: &Paths, state: &State, format: Format) -> Result<ExitCode> {
    use std::io::Write;
    let cfg = Config::load(paths)?;
    let site = state
        .get_or(state::keys::BOORU_SITE, &cfg.booru.default_site)
        .to_string();
    let query = state.get_or(state::keys::BOORU_QUERY, "").to_string();
    let page: u32 = state
        .get_or(state::keys::BOORU_PAGE, "1")
        .parse()
        .unwrap_or(1);
    let search_mode = state.get_or(state::keys::BOORU_SEARCH_MODE, "").to_string();

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    if search_mode == "on" {
        let rows = [Row::Control {
            label: "← Cancel".to_string(),
            info: "booru:cancel-search".to_string(),
            icon: None,
        }];
        let hints = ViewHints {
            prompt: format!("Search {site}"),
            message: "Type tags (space-separated), Enter to search. Esc cancels.".to_string(),
            use_hot_keys: true,
            // Let the user's typed query come through as `$selection` —
            // there is no row to select for free-form text otherwise.
            allow_custom: true,
            // Best-effort: rofi's `filter` header only pre-fills the input
            // on the initial mode launch, not on script callbacks (which is
            // how Alt+2 lands here). Emit it anyway — harmless, and the rare
            // rofi build that honors it on re-invocation benefits.
            filter: query.clone(),
        };
        write_rows(&mut out, &rows, &hints, format)?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    // Cached results from the last `wallrack booru search`. read_index for
    // booru returns the cached page (or an empty index when nothing's been
    // searched yet).
    let integ = integrations::by_name("booru")?;
    let index = integ.read_index(paths)?;

    // Search (Alt+2), site switch (Alt+6), and pagination (Alt+7/Alt+8) are
    // all hotkeys — no control rows. Keeping them out keeps the post grid
    // unbroken and stops accidental Enter on a row two slots above the post
    // the user actually meant to pick.
    let mut rows: Vec<Row<'_>> = Vec::with_capacity(index.entries.len().max(1));
    for e in &index.entries {
        rows.push(Row::Entry {
            entry: e,
            favorite: false,
            label: None,
            // `booru-post:` keeps it distinct from `image:<path>` so the rofi
            // wrapper can route us to download-then-monitor rather than
            // straight to a wallpaper apply.
            info: Some(format!("booru-post:{}", e.id)),
        });
    }
    // Rofi script-mode closes when it receives zero rows. A no-result search
    // (or first-run empty state) still has to feed it at least one placeholder
    // so the user keeps the view and can Alt+2 again. `noop:*` is the picker's
    // existing inert-row prefix — Enter on it just re-renders.
    if rows.is_empty() {
        let label = if query.is_empty() {
            format!("No search yet on {site} — Alt+2 to search")
        } else {
            format!("No results for `{query}` on {site} — Alt+2 / Alt+6 to retry")
        };
        rows.push(Row::Control {
            label,
            info: "noop:booru-empty".to_string(),
            icon: None,
        });
    }

    let prompt = if query.is_empty() {
        format!("booru/{site}")
    } else {
        format!("booru/{site} · {query} · p{page}")
    };
    let message = if index.entries.is_empty() && query.is_empty() {
        "Alt+2 search · Alt+6 cycle site · Alt+7/Alt+8 page".to_string()
    } else if index.entries.is_empty() {
        format!("No results for `{query}` on {site}. Alt+2 to retype.")
    } else {
        format!(
            "{} results — Enter download + apply · Alt+2 search · Alt+6 site · Alt+7/Alt+8 page",
            index.entries.len()
        )
    };
    let hints = ViewHints {
        prompt,
        message,
        use_hot_keys: true,
        allow_custom: false,
        filter: String::new(),
    };
    write_rows(&mut out, &rows, &hints, format)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Render the per-entry tag editor. Rows: a Back row, an Add row, and one
/// row per tag currently on the entry — selecting a tag row removes it.
fn cmd_tag_editor_view(
    paths: &Paths,
    integration: &str,
    target: &str,
    format: Format,
) -> Result<ExitCode> {
    let integ = integrations::by_name(integration)?;
    let idx = integ.read_index(paths)?;
    let tags: Vec<String> = idx
        .entries
        .iter()
        .find(|e| e.id == target)
        .map(|e| e.tags.clone())
        .unwrap_or_default();
    let label = target.rsplit('/').next().unwrap_or(target).to_string();

    let mut rows: Vec<Row<'_>> = Vec::with_capacity(tags.len() + 2);
    rows.push(Row::Control {
        label: "← Back".to_string(),
        info: "tagedit:back".to_string(),
        icon: None,
    });
    rows.push(Row::Control {
        label: "+ Add tag…".to_string(),
        info: "tagedit:add".to_string(),
        icon: None,
    });
    for t in &tags {
        if t.is_empty() {
            continue;
        }
        rows.push(Row::Control {
            label: t.clone(),
            info: format!("tagedit:remove:{t}"),
            icon: None,
        });
    }
    let hints = ViewHints {
        prompt: format!("Tags: {label}"),
        message: "Enter to remove tag | \"+ Add\" prompts for a new tag | ← Back".to_string(),
        use_hot_keys: true,
        allow_custom: false,
        filter: String::new(),
    };
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    write_rows(&mut out, &rows, &hints, format)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Render the add-tag prompt. Rows: a Cancel row + every catalog tag for
/// the active integration. Wrappers that support free-form input let the
/// user type a brand-new tag too.
fn cmd_add_tag_view(
    paths: &Paths,
    integration: &str,
    target: &str,
    format: Format,
) -> Result<ExitCode> {
    let catalog = crate::tags::TagCatalog::load(&paths.tag_catalog_file())?;
    let tags = catalog.list(integration);
    let label = target.rsplit('/').next().unwrap_or(target).to_string();

    let mut rows: Vec<Row<'_>> = Vec::with_capacity(tags.len() + 1);
    rows.push(Row::Control {
        label: "← Cancel".to_string(),
        info: "tagedit:cancel".to_string(),
        icon: None,
    });
    for t in &tags {
        if t.is_empty() {
            continue;
        }
        rows.push(Row::Control {
            label: t.clone(),
            // The rofi script treats a non-`tagedit:*` info as "user picked a
            // catalog tag to add" — same convention here for any wrapper.
            info: format!("tagedit:pick:{t}"),
            icon: None,
        });
    }
    let hints = ViewHints {
        prompt: format!("Add tag to {label}"),
        message: "Pick a known tag or type a new one — Enter to add, Esc to cancel".to_string(),
        use_hot_keys: true,
        // Free-form tag entry — `$selection` carries the typed string.
        allow_custom: true,
        filter: String::new(),
    };
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    write_rows(&mut out, &rows, &hints, format)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

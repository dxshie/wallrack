//! `wallrack view` — render the picker view based on persisted state.
//! Dispatches to one of several sub-views via the [`PickerView`] enum.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::Result;

use crate::config::Config;
use crate::favorites::Favorites;
use crate::integrations;
use crate::output::{Action, Format, Row, ViewHints, write_rows};
use crate::paths::Paths;
use crate::state::{PickerView, State};

use super::super::render::{
    emit_drill_view, emit_empty_view, emit_flat, emit_grouped_view, filter_entries,
};
use super::tags;

pub(in crate::cli) fn run(paths: &Paths, format: Format) -> Result<ExitCode> {
    let state = State::open(paths.store())?;
    let integration = state.picker_mode().as_str().to_string();
    let tag_edit_target = state.tag_edit_target();

    match state.picker_view() {
        PickerView::TagAdd => cmd_add_tag_view(paths, &integration, &tag_edit_target, format),
        PickerView::TagEditor => {
            cmd_tag_editor_view(paths, &integration, &tag_edit_target, format)
        }
        PickerView::TagSelect => tags::run(paths, Some(&integration), format),
        PickerView::Booru => cmd_booru_view(paths, &state, format),
        PickerView::Default => default_view(paths, &state, &integration, format),
    }
}

fn default_view(
    paths: &Paths,
    state: &State,
    integration: &str,
    format: Format,
) -> Result<ExitCode> {
    let integ = integrations::by_name(integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::open(paths.store())?;

    let favorites_only = state.view_mode().favorites_only();
    let tag_filter = state.tag_filter();
    let tag = (!tag_filter.is_empty()).then_some(tag_filter.as_str());
    let rating_opt = state.rating_filter().as_filter();
    let drill = state.drill_path();
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
        emit_empty_view(&mut out, integration, favorites_only, tag, format)?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(folder_path) = folder_opt {
        emit_drill_view(
            &mut out,
            &filtered,
            &favorites,
            integration,
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
            integration,
            favorites_only,
            tag,
            format,
        )?;
    } else {
        emit_flat(
            &mut out,
            &filtered,
            &favorites,
            integration,
            favorites_only,
            tag,
            format,
        )?;
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Render the booru picker view. Two sub-views:
///   * `booru_search_mode = on` → search prompt (single Cancel row, picker
///     uses rofi's allow-custom to capture the typed query).
///   * normal → control rows (Search / Site / Prev / Next) followed by the
///     cached posts from the last `wallrack booru search`.
fn cmd_booru_view(paths: &Paths, state: &State, format: Format) -> Result<ExitCode> {
    let cfg = Config::load(paths)?;
    let site = state
        .booru_site()
        .unwrap_or_else(|| cfg.booru.default_site.clone());
    let query = state.booru_query().to_string();
    let page = state.booru_page();

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    if state.booru_search_mode_on() {
        let rows = [Row::Control {
            label: "← Cancel".to_string(),
            action: Action::BooruCancelSearch,
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
            // BooruPost keeps it distinct from ApplyImage so the rofi
            // wrapper can route us to download-then-monitor rather than
            // straight to a wallpaper apply.
            action: Some(Action::BooruPost { id: e.id().to_string() }),
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
            action: Action::Noop { reason: "booru-empty".into() },
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
        .find(|e| e.id() == target)
        .map(|e| e.tags().to_vec())
        .unwrap_or_default();
    let label = target.rsplit('/').next().unwrap_or(target).to_string();

    let mut rows: Vec<Row<'_>> = Vec::with_capacity(tags.len() + 2);
    rows.push(Row::Control {
        label: "← Back".to_string(),
        action: Action::TagEditBack,
        icon: None,
    });
    rows.push(Row::Control {
        label: "+ Add tag…".to_string(),
        action: Action::TagEditAdd,
        icon: None,
    });
    for t in &tags {
        if t.is_empty() {
            continue;
        }
        rows.push(Row::Control {
            label: t.clone(),
            action: Action::TagEditRemove { tag: t.clone() },
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
    let catalog = crate::tags::TagCatalog::open(paths.store())?;
    let tags = catalog.list(integration);
    let label = target.rsplit('/').next().unwrap_or(target).to_string();

    let mut rows: Vec<Row<'_>> = Vec::with_capacity(tags.len() + 1);
    rows.push(Row::Control {
        label: "← Cancel".to_string(),
        action: Action::TagEditCancel,
        icon: None,
    });
    for t in &tags {
        if t.is_empty() {
            continue;
        }
        rows.push(Row::Control {
            label: t.clone(),
            action: Action::TagEditPick { tag: t.clone() },
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

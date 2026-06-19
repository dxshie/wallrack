//! View rendering helpers — shared by `list` and `view` commands. Every
//! function here is a pure transformation from "filter context + entries"
//! to "rows + hints" handed off to the format writer.

use std::collections::BTreeSet;
use std::io::Write;

use anyhow::Result;

use crate::entry::{Entry, Index};
use crate::favorites::Favorites;
use crate::integrations;
use crate::output::{Action, Format, Row, ViewHints, write_rows};

pub(super) fn filter_entries<'a>(
    index: &'a Index,
    favorites: &Favorites,
    favorites_only: bool,
    tag: Option<&str>,
    rating: Option<&str>,
    folder: Option<&str>,
) -> Vec<&'a Entry> {
    index
        .entries
        .iter()
        .filter(|e| {
            if favorites_only && !favorites.is_favorite(&e.integration, &e.id) {
                return false;
            }
            if let Some(t) = tag {
                if !e.tags.iter().any(|x| x == t) {
                    return false;
                }
            }
            if let Some(r) = rating {
                if !r.is_empty() && r != "All" && e.rating != r {
                    return false;
                }
            }
            if let Some(f) = folder {
                // Match images that live directly inside `f` (trailing slash trimmed).
                let want = f.trim_end_matches('/');
                let parent = e
                    .source
                    .parent()
                    .map(|p| p.to_string_lossy().trim_end_matches('/').to_string())
                    .unwrap_or_default();
                if parent != want {
                    return false;
                }
            }
            true
        })
        .collect()
}

pub(super) fn emit_flat<W: Write>(
    w: &mut W,
    entries: &[&Entry],
    favorites: &Favorites,
    integration: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    // Image-based entries carry their id explicitly so the shell doesn't
    // have to recover it by string-splitting the display line — file paths
    // like "foo - bar.jpg" would break that on the last " - ". The `we`
    // integration uses folder paths (no " - " in workshop ids) so the
    // string-split fallback is safe and we don't override its action.
    let is_image = integration == "wallpaper" || integration == "we_image";
    let rows: Vec<Row<'_>> = entries
        .iter()
        .map(|e| Row::Entry {
            entry: e,
            favorite: favorites.is_favorite(&e.integration, &e.id),
            label: None,
            action: is_image.then(|| Action::ApplyImage { id: e.id.clone() }),
        })
        .collect();
    write_rows(
        w,
        &rows,
        &view_hints_for(integration, None, favorites_only, tag_filter),
        format,
    )
}

pub(super) fn emit_drill_view<W: Write>(
    w: &mut W,
    entries: &[&Entry],
    favorites: &Favorites,
    integration: &str,
    folder_path: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    let mut rows: Vec<Row<'_>> = Vec::with_capacity(entries.len() + 1);
    rows.push(Row::Control {
        label: "← Back".to_string(),
        action: Action::Back,
        icon: None,
    });
    for e in entries {
        let label = e
            .source
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| e.title.clone());
        rows.push(Row::Entry {
            entry: e,
            favorite: favorites.is_favorite(&e.integration, &e.id),
            label: Some(label),
            action: Some(Action::ApplyImage { id: e.id.clone() }),
        });
    }
    let mut hints = view_hints_for(integration, Some(folder_path), favorites_only, tag_filter);
    hints.message = "Alt+3 fav | Alt+2 tag | select ← Back to return".to_string();
    write_rows(w, &rows, &hints, format)
}

pub(super) fn emit_grouped_view<W: Write>(
    w: &mut W,
    entries: &[&Entry],
    favorites: &Favorites,
    integration: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    let mut rows: Vec<Row<'_>> = Vec::new();
    let mut seen_folders: BTreeSet<String> = BTreeSet::new();

    for e in entries {
        if e.subfolder.is_empty() {
            // Root-level: emit as individual entry. ApplyImage with the
            // entry id is what the shell needs — paths containing " - "
            // would otherwise be mis-split by display-text parsing.
            rows.push(Row::Entry {
                entry: e,
                favorite: favorites.is_favorite(&e.integration, &e.id),
                label: None,
                action: Some(Action::ApplyImage { id: e.id.clone() }),
            });
        } else {
            // Nested: emit one entry per (workshop_id, subfolder).
            let key = format!(
                "{}\u{1c}{}",
                e.workshop_id.clone().unwrap_or_default(),
                e.subfolder
            );
            if !seen_folders.insert(key) {
                continue;
            }
            let folder_path = e
                .source
                .parent()
                .map(|p| format!("{}/", p.to_string_lossy()))
                .unwrap_or_default();
            rows.push(Row::Entry {
                entry: e,
                favorite: false, // folders aren't favoritable
                label: Some(format!("{} - {}", e.title, e.subfolder)),
                action: Some(Action::Drill { folder: folder_path }),
            });
        }
    }
    write_rows(
        w,
        &rows,
        &view_hints_for(integration, None, favorites_only, tag_filter),
        format,
    )
}

/// Render a placeholder row when the current view would otherwise produce
/// zero entries. Rofi exits as soon as the script writes no rows, which
/// closes the picker abruptly — typically right after Alt+1 lands on an
/// integration that's unindexed or has an empty config. This keeps rofi
/// open and steers the user toward the keys that fix it.
pub(super) fn emit_empty_view<W: Write>(
    w: &mut W,
    integration: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    let label = integrations::by_name(integration)
        .ok()
        .map(|i| i.label().to_string())
        .unwrap_or_else(|| integration.to_string());
    let index_empty = !favorites_only && tag_filter.map(|t| t.is_empty()).unwrap_or(true);
    let reason = if favorites_only {
        format!("No favorited {label} yet — Alt+3 on an entry to favorite it")
    } else if tag_filter.map(|t| !t.is_empty()).unwrap_or(false) {
        format!("No {label} match the current tag filter — Alt+2 to clear")
    } else {
        format!("No {label} indexed")
    };
    // Only suggest config edits when the *index itself* is empty — for an
    // empty favorites or tag-filter view, the index might be fine; the
    // filter is just too narrow.
    let hint = if index_empty {
        match integration {
            "wallpaper" => " — set `wallpaper.dirs` in config.toml then press Alt+0",
            "we_image" | "we" => " — check `workshop_dir` in config.toml then press Alt+0",
            _ => " — press Alt+0 to refresh or Alt+1 to switch mode",
        }
    } else {
        ""
    };
    let row = Row::Control {
        label: format!("{reason}{hint}"),
        action: Action::Noop { reason: "empty".into() },
        icon: None,
    };
    let hints = view_hints_for(integration, None, favorites_only, tag_filter);
    write_rows(w, &[row], &hints, format)
}

pub(super) fn view_hints_for(
    integration: &str,
    drill: Option<&str>,
    favorites_only: bool,
    tag_filter: Option<&str>,
) -> ViewHints {
    let base = match drill {
        Some(d) => folder_label(d),
        None => integrations::by_name(integration)
            .map(|i| i.label().to_string())
            .unwrap_or_else(|_| "Wallpapers".to_string()),
    };
    let mut prompt = if favorites_only {
        format!("★ {base}")
    } else {
        base
    };
    if let Some(t) = tag_filter {
        if !t.is_empty() {
            prompt = format!("{prompt} #{t}");
        }
    }
    ViewHints {
        prompt,
        message: "Alt+1 mode | Alt+2 tag | Alt+3 fav | Alt+4 view | Alt+5 edit tags | Alt+6 rating | Alt+0 refresh".to_string(),
        use_hot_keys: true,
        allow_custom: false,
        filter: String::new(),
    }
}

pub(super) fn folder_label(folder_path: &str) -> String {
    folder_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(folder_path)
        .to_string()
}

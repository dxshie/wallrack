//! Shared "resolve picker context from state" helpers — the original CLI
//! repeated the same `Option<&str> → persisted picker_mode → "wallpaper"`
//! fallback in half a dozen commands.

use anyhow::{Result, anyhow};

use crate::entry::Entry;
use crate::integrations;
use crate::paths::Paths;
use crate::state::State;

/// Resolve the active integration: explicit override → persisted picker
/// mode → `"wallpaper"`.
pub(super) fn resolve_integration(paths: &Paths, override_: Option<&str>) -> Result<String> {
    if let Some(s) = override_ {
        return Ok(s.to_string());
    }
    let state = State::open(paths.store())?;
    Ok(state.picker_mode().as_str().to_string())
}

/// Validate that exactly one of `id` / `folder` was provided on a
/// `(--id|--folder)` command. clap's `conflicts_with` already rules out
/// "both" — the runtime check here covers the "neither" case.
pub(super) fn require_one_target<'a>(
    id: Option<&'a str>,
    folder: Option<&'a str>,
) -> Result<TargetSpec<'a>> {
    match (id, folder) {
        (Some(i), None) => Ok(TargetSpec::Id(i)),
        (None, Some(f)) => Ok(TargetSpec::Folder(f)),
        (None, None) => Err(anyhow!("pass exactly one of --id or --folder")),
        (Some(_), Some(_)) => Err(anyhow!("--id and --folder are mutually exclusive")),
    }
}

pub(super) enum TargetSpec<'a> {
    Id(&'a str),
    Folder(&'a str),
}

/// Enumerate entry ids whose source's parent path equals `folder` (trailing
/// slash trimmed). Used by `--folder` variants of tag / rating / favorites
/// commands to fan a single edit out across every image in a drillable
/// subfolder. Empty result is not an error — the caller decides.
pub(super) fn entries_in_folder(
    paths: &Paths,
    integration: &str,
    folder: &str,
) -> Result<Vec<Entry>> {
    let integ = integrations::by_name(integration)?;
    let idx = integ.read_index(paths)?;
    let want = folder.trim_end_matches('/');
    Ok(idx
        .entries
        .into_iter()
        .filter(|e| {
            e.source()
                .parent()
                .map(|p| p.to_string_lossy().trim_end_matches('/').to_string())
                .as_deref()
                == Some(want)
        })
        .collect())
}

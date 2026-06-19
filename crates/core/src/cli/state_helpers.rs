//! Shared "resolve picker context from state" helpers — the original CLI
//! repeated the same `Option<&str> → persisted picker_mode → "wallpaper"`
//! fallback in half a dozen commands.

use anyhow::Result;

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

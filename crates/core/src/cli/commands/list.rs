use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::Result;

use crate::favorites::Favorites;
use crate::integrations;
use crate::output::Format;
use crate::paths::Paths;
use crate::state::State;

use super::super::render::{
    emit_drill_view, emit_empty_view, emit_flat, emit_grouped_view, filter_entries,
};

pub(in crate::cli) struct ListArgs {
    pub integration: String,
    pub favorites_only: bool,
    pub tag: Option<String>,
    pub rating: Option<String>,
    pub folder: Option<String>,
    pub use_state: bool,
    pub group: bool,
    pub format: Format,
}

pub(in crate::cli) fn run(paths: &Paths, args: ListArgs) -> Result<ExitCode> {
    let (integration, favorites_only, tag, rating, folder, group) = if args.use_state {
        // Pull filter context from persisted picker state.
        let state = State::load(&paths.state_file())?;
        let drill = state.drill_path().to_string();
        let tag_filter = state.tag_filter().to_string();
        let group = drill.is_empty(); // group at top level, flat inside a folder
        (
            state.picker_mode().as_str().to_string(),
            state.view_mode().favorites_only(),
            (!tag_filter.is_empty()).then_some(tag_filter),
            state.rating_filter().as_filter().map(str::to_string),
            (!drill.is_empty()).then_some(drill),
            group,
        )
    } else {
        (
            args.integration,
            args.favorites_only,
            args.tag,
            args.rating,
            args.folder,
            args.group,
        )
    };

    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::load(&paths.favorites_file())?;

    let filtered = filter_entries(
        &index,
        &favorites,
        favorites_only,
        tag.as_deref(),
        rating.as_deref(),
        folder.as_deref(),
    );

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    if filtered.is_empty() && folder.is_none() {
        emit_empty_view(
            &mut out,
            &integration,
            favorites_only,
            tag.as_deref(),
            args.format,
        )?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(folder_path) = folder.as_deref() {
        emit_drill_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            folder_path,
            favorites_only,
            tag.as_deref(),
            args.format,
        )?;
    } else if group && integ.supports_drill() {
        emit_grouped_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag.as_deref(),
            args.format,
        )?;
    } else {
        emit_flat(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag.as_deref(),
            args.format,
        )?;
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

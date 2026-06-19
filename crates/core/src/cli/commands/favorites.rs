use std::io;
use std::process::ExitCode;

use anyhow::Result;

use crate::favorites::Favorites;
use crate::paths::Paths;

use super::super::args::{FavoritesCmd, IntegrationArg};
use super::super::format_list::write_string_list;
use super::super::state_helpers::resolve_integration;

pub(in crate::cli) fn run(paths: &Paths, cmd: FavoritesCmd) -> Result<ExitCode> {
    let favorites = Favorites::open(paths.store())?;
    match cmd {
        FavoritesCmd::List {
            integration,
            format,
        } => {
            let integration =
                resolve_integration(paths, integration.map(IntegrationArg::as_str))?;
            let ids = favorites.list(&integration);
            let stdout = io::stdout().lock();
            let mut out = std::io::BufWriter::new(stdout);
            write_string_list(&mut out, &ids, format)?;
            use std::io::Write;
            out.flush()?;
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Add { integration, id } => {
            favorites.add(integration.as_str(), &id);
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Remove { integration, id } => {
            favorites.remove(integration.as_str(), &id);
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Toggle { integration, id } => {
            let now_fav = favorites.toggle(integration.as_str(), &id);
            println!("{}", if now_fav { "added" } else { "removed" });
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Is { integration, id } => Ok(
            if favorites.is_favorite(integration.as_str(), &id) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            },
        ),
    }
}

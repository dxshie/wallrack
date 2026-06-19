use std::io;
use std::process::ExitCode;

use anyhow::{Result, anyhow};

use crate::favorites::Favorites;
use crate::paths::Paths;

use super::super::args::{FavoritesCmd, IntegrationArg};
use super::super::format_list::write_string_list;
use super::super::state_helpers::{
    TargetSpec, entries_in_folder, require_one_target, resolve_integration,
};

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
        FavoritesCmd::Add {
            integration,
            id,
            folder,
        } => {
            match require_one_target(id.as_deref(), folder.as_deref())? {
                TargetSpec::Id(id) => {
                    favorites.add(integration.as_str(), id);
                }
                TargetSpec::Folder(folder) => {
                    let entries = entries_in_folder(paths, integration.as_str(), folder)?;
                    if entries.is_empty() {
                        return Err(anyhow!("no entries under folder {folder}"));
                    }
                    for e in &entries {
                        favorites.add(integration.as_str(), e.id());
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Remove {
            integration,
            id,
            folder,
        } => {
            match require_one_target(id.as_deref(), folder.as_deref())? {
                TargetSpec::Id(id) => {
                    favorites.remove(integration.as_str(), id);
                }
                TargetSpec::Folder(folder) => {
                    let entries = entries_in_folder(paths, integration.as_str(), folder)?;
                    for e in &entries {
                        favorites.remove(integration.as_str(), e.id());
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Toggle {
            integration,
            id,
            folder,
        } => {
            match require_one_target(id.as_deref(), folder.as_deref())? {
                TargetSpec::Id(id) => {
                    let now_fav = favorites.toggle(integration.as_str(), id);
                    println!("{}", if now_fav { "added" } else { "removed" });
                }
                TargetSpec::Folder(folder) => {
                    let entries = entries_in_folder(paths, integration.as_str(), folder)?;
                    if entries.is_empty() {
                        return Err(anyhow!("no entries under folder {folder}"));
                    }
                    // Collective toggle: if everything in the folder is
                    // already favorited, drop the lot; otherwise upgrade the
                    // missing ones so the whole folder ends up favorited.
                    let all_fav = entries
                        .iter()
                        .all(|e| favorites.is_favorite(integration.as_str(), e.id()));
                    if all_fav {
                        for e in &entries {
                            favorites.remove(integration.as_str(), e.id());
                        }
                        println!("removed");
                    } else {
                        for e in &entries {
                            favorites.add(integration.as_str(), e.id());
                        }
                        println!("added");
                    }
                }
            }
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

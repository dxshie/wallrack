use std::process::ExitCode;

use anyhow::{Result, anyhow};

use crate::integrations;
use crate::paths::Paths;

use super::super::args::RatingCmd;
use super::super::state_helpers::{TargetSpec, entries_in_folder, require_one_target};

pub(in crate::cli) fn run(paths: &Paths, cmd: RatingCmd) -> Result<ExitCode> {
    let overrides = crate::rating::RatingOverrides::open(paths.store())?;
    match cmd {
        RatingCmd::Set {
            integration,
            id,
            folder,
            rating,
        } => match require_one_target(id.as_deref(), folder.as_deref())? {
            TargetSpec::Id(id) => {
                overrides.set(integration.as_str(), id, rating)?;
            }
            TargetSpec::Folder(folder) => {
                let entries = entries_in_folder(paths, integration.as_str(), folder)?;
                if entries.is_empty() {
                    return Err(anyhow!("no entries under folder {folder}"));
                }
                for e in &entries {
                    overrides.set(integration.as_str(), e.id(), rating)?;
                }
            }
        },
        RatingCmd::Clear {
            integration,
            id,
            folder,
        } => match require_one_target(id.as_deref(), folder.as_deref())? {
            TargetSpec::Id(id) => {
                overrides.clear(integration.as_str(), id)?;
            }
            TargetSpec::Folder(folder) => {
                let entries = entries_in_folder(paths, integration.as_str(), folder)?;
                for e in &entries {
                    overrides.clear(integration.as_str(), e.id())?;
                }
            }
        },
        RatingCmd::Show { integration, id } => {
            let integ = integrations::by_name(integration.as_str())?;
            let idx = integ.read_index(paths)?;
            if let Some(entry) = idx.entries.iter().find(|e| e.id() == id) {
                println!("{}", entry.rating());
            } else {
                return Err(anyhow!("entry not in index: {id}"));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

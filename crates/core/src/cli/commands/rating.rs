use std::process::ExitCode;

use anyhow::{Result, anyhow};

use crate::integrations;
use crate::paths::Paths;

use super::super::args::RatingCmd;

pub(in crate::cli) fn run(paths: &Paths, cmd: RatingCmd) -> Result<ExitCode> {
    let overrides = crate::rating::RatingOverrides::open(paths.store())?;
    match cmd {
        RatingCmd::Set {
            integration,
            id,
            rating,
        } => {
            overrides.set(integration.as_str(), &id, rating)?;
        }
        RatingCmd::Clear { integration, id } => {
            overrides.clear(integration.as_str(), &id)?;
        }
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

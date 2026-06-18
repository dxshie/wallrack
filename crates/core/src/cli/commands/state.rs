use std::io;
use std::process::ExitCode;

use anyhow::Result;

use crate::paths::Paths;
use crate::state::{self, State};

use super::super::args::StateCmd;

pub(in crate::cli) fn run(paths: &Paths, cmd: StateCmd) -> Result<ExitCode> {
    let state_path = paths.state_file();
    let mut state = State::load(&state_path)?;
    match cmd {
        StateCmd::Get { key } => {
            if let Some(v) = state.get(&key) {
                println!("{v}");
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
        StateCmd::Set { key, value } => {
            state.set(&key, value);
            state.save(&state_path)?;
            Ok(ExitCode::SUCCESS)
        }
        StateCmd::Unset { key } => {
            state.remove(&key);
            state.save(&state_path)?;
            Ok(ExitCode::SUCCESS)
        }
        StateCmd::Dump => {
            let stdout = io::stdout().lock();
            serde_json::to_writer_pretty(stdout, state.all())?;
            println!();
            Ok(ExitCode::SUCCESS)
        }
        StateCmd::ResetTransient => {
            state.remove(state::keys::DRILL_PATH);
            state.remove(state::keys::TAG_MODE);
            state.remove(state::keys::TAG_EDIT_TARGET);
            state.remove(state::keys::TAG_ADD_MODE);
            state.remove(state::keys::BOORU_SEARCH_MODE);
            state.remove(state::keys::APPLY_INTEGRATION_OVERRIDE);
            state.save(&state_path)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

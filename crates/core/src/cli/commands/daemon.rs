use std::process::ExitCode;

use anyhow::Result;

use crate::config::Config;
use crate::daemon::Daemon;
use crate::paths::Paths;

use super::super::args::DaemonCmd;

pub(in crate::cli) fn run(paths: &Paths, config: &Config, cmd: DaemonCmd) -> Result<ExitCode> {
    let d = Daemon::new(paths);
    match cmd {
        DaemonCmd::Start { foreground } => {
            d.start(config, foreground)?;
        }
        DaemonCmd::Stop => {
            d.stop()?;
        }
        DaemonCmd::Status => {
            d.status()?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

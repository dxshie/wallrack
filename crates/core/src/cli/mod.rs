//! `wallrack` command-line interface. Argument parsing lives in [`args`];
//! per-command implementations live in [`commands`]; shared helpers (render,
//! state-resolution, hook-running, terminal styling, notification, format
//! dispatch) live in their named modules. This module wires the arg-parser
//! to the implementations and does nothing else.

use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser};

use crate::config::Config;
use crate::paths::Paths;

mod args;
mod commands;
mod format_list;
mod hooks;
mod notify;
mod render;
mod state_helpers;
mod style;

use args::{Cmd, IntegrationArg};

#[derive(Parser)]
#[command(name = "wallrack", version, about = "Modular wallpaper manager")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

pub fn run() -> Result<ExitCode> {
    let matches = Cli::command()
        .styles(style::make_clap_styles())
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let paths = Paths::discover()?;
    let config = Config::load(&paths)?;

    match cli.cmd {
        Cmd::Index { integration } => commands::index::run(&paths, &config, integration.as_str()),
        Cmd::List {
            integration,
            format,
            favorites,
            tag,
            rating,
            folder,
            use_state,
            group,
        } => commands::list::run(
            &paths,
            commands::list::ListArgs {
                integration: integration.as_str().to_string(),
                format,
                favorites_only: favorites,
                tag,
                rating,
                folder,
                use_state,
                group,
            },
        ),
        Cmd::View { format } => commands::view::run(&paths, format),
        Cmd::Tags {
            integration,
            format,
        } => commands::tags::run(&paths, integration.map(IntegrationArg::as_str), format),
        Cmd::Tag { cmd } => commands::tag::run(&paths, cmd),
        Cmd::Rating { cmd } => commands::rating::run(&paths, cmd),
        Cmd::Favorites { cmd } => commands::favorites::run(&paths, cmd),
        Cmd::State { cmd } => commands::state::run(&paths, cmd),
        Cmd::Monitors {
            integration,
            target,
            format,
        } => commands::monitors::run(
            &paths,
            &config,
            integration.map(IntegrationArg::as_str),
            target.as_deref(),
            format,
        ),
        Cmd::Apply {
            integration,
            monitor,
            target,
        } => commands::apply::run(
            &paths,
            &config,
            integration.map(IntegrationArg::as_str),
            &monitor,
            &target,
        ),
        Cmd::Daemon { cmd } => commands::daemon::run(&paths, &config, cmd),
        Cmd::Booru { cmd } => commands::booru::run(&paths, &config, cmd),
        Cmd::Info => commands::info::run(&paths, &config),
    }
}

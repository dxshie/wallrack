use std::process::ExitCode;

mod cli;
mod config;
mod daemon;
mod entry;
mod favorites;
mod integrations;
mod output;
mod paths;
mod state;
mod tags;
mod thumbnail;

fn main() -> ExitCode {
    match cli::run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("wallrack: {err:#}");
            ExitCode::from(1)
        }
    }
}

use std::process::ExitCode;

mod cli;
mod config;
mod daemon;
mod entry;
mod favorites;
mod integrations;
mod output;
mod paths;
mod rating;
mod state;
mod tags;
mod thumbnail;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            match record.level() {
                log::Level::Error => writeln!(buf, "wallrack: error: {}", record.args()),
                log::Level::Warn  => writeln!(buf, "wallrack: warning: {}", record.args()),
                log::Level::Info  => writeln!(buf, "wallrack: {}", record.args()),
                log::Level::Debug => writeln!(buf, "wallrack: debug: {}", record.args()),
                log::Level::Trace => writeln!(buf, "wallrack: trace: {}", record.args()),
            }
        })
        .init();

    match cli::run() {
        Ok(code) => code,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::from(1)
        }
    }
}

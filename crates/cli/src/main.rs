use std::process::ExitCode;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            match record.level() {
                log::Level::Error => writeln!(buf, "wallrack: error: {}", record.args()),
                log::Level::Warn => writeln!(buf, "wallrack: warning: {}", record.args()),
                log::Level::Info => writeln!(buf, "wallrack: {}", record.args()),
                log::Level::Debug => writeln!(buf, "wallrack: debug: {}", record.args()),
                log::Level::Trace => writeln!(buf, "wallrack: trace: {}", record.args()),
            }
        })
        .init();

    match wallrack_core::cli::run() {
        Ok(code) => code,
        Err(err) => {
            log::error!("{err:#}");
            ExitCode::from(1)
        }
    }
}

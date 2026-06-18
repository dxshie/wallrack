//! Terminal coloring and clap styling — shared by every command that
//! writes user-facing TTY output.

use std::io::{self, IsTerminal};

pub(super) struct C {
    pub bold: &'static str,
    pub green: &'static str,
    pub yellow: &'static str,
    pub cyan: &'static str,
    pub red: &'static str,
    pub dim: &'static str,
    pub reset: &'static str,
}

impl C {
    pub fn stdout() -> Self {
        Self::for_tty(io::stdout().is_terminal())
    }
    pub fn stderr() -> Self {
        Self::for_tty(io::stderr().is_terminal())
    }
    fn for_tty(on: bool) -> Self {
        if on {
            Self {
                bold: "\x1b[1m",
                green: "\x1b[32m",
                yellow: "\x1b[33m",
                cyan: "\x1b[36m",
                red: "\x1b[31m",
                dim: "\x1b[2m",
                reset: "\x1b[0m",
            }
        } else {
            Self {
                bold: "",
                green: "",
                yellow: "",
                cyan: "",
                red: "",
                dim: "",
                reset: "",
            }
        }
    }
}

pub(super) fn make_clap_styles() -> clap::builder::Styles {
    use clap::builder::styling::{AnsiColor, Effects, Styles};
    Styles::styled()
        .header(AnsiColor::Yellow.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::White.on_default())
        .error(AnsiColor::Red.on_default() | Effects::BOLD)
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Yellow.on_default())
}

//! wallrack-core — picker-agnostic wallpaper manager library.
//!
//! Indexes wallpaper sources (filesystem directories, Steam Workshop, booru
//! search APIs), persists user-applied state (favorites, tag overrides,
//! ratings, picker state), and renders entries through a format-agnostic row
//! pipeline (rofi / dmenu / JSON).
//!
//! The `wallrack` binary is a thin wrapper that initializes logging and calls
//! [`cli::run`].

pub mod cli;
pub mod config;
pub mod daemon;
pub mod entry;
pub mod favorites;
pub mod integrations;
pub mod output;
pub mod paths;
pub mod rating;
pub mod state;
pub mod tags;
pub mod thumbnail;

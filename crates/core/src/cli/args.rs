//! Clap-derived argument types for the wallrack CLI. The `Cmd` enum and its
//! sub-Cmd enums live here; the actual command implementations live under
//! [`super::commands`] and the dispatch sits in [`super::run`].

use clap::{Subcommand, ValueEnum};

use crate::output::Format;

/// CLI surface for selecting an integration. The string values match the
/// on-disk integration keys used everywhere else (cache dirs, state, the
/// picker shell), so we can hand `as_str()` straight to `integrations::by_name`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum IntegrationArg {
    /// Plain images from `wallpaper.dirs` in config.toml.
    #[value(name = "wallpaper")]
    Wallpaper,
    /// Images extracted from Wallpaper Engine workshop projects.
    #[value(name = "we_image")]
    WeImage,
    /// Live Wallpaper Engine projects (linux-wallpaperengine).
    #[value(name = "we")]
    We,
    /// Danbooru-style image board search (konachan, yandere, danbooru, …).
    #[value(name = "booru")]
    Booru,
}

impl IntegrationArg {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Wallpaper => "wallpaper",
            Self::WeImage => "we_image",
            Self::We => "we",
            Self::Booru => "booru",
        }
    }
}

/// Like [`IntegrationArg`] but with an extra `all` value for `wallrack index`,
/// which can rebuild every integration in one go.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum IndexTarget {
    /// Rebuild every integration's index.
    All,
    #[value(name = "wallpaper")]
    Wallpaper,
    #[value(name = "we_image")]
    WeImage,
    #[value(name = "we")]
    We,
    #[value(name = "booru")]
    Booru,
}

impl IndexTarget {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Wallpaper => "wallpaper",
            Self::WeImage => "we_image",
            Self::We => "we",
            Self::Booru => "booru",
        }
    }
}

#[derive(Subcommand)]
pub(super) enum Cmd {
    /// Build the on-disk index for one or all integrations.
    Index {
        #[arg(long, value_enum, default_value_t = IndexTarget::All)]
        integration: IndexTarget,
    },
    /// Emit wallpapers in the requested format.
    List {
        #[arg(long, value_enum, default_value_t = IntegrationArg::Wallpaper)]
        integration: IntegrationArg,
        #[arg(long, default_value = "rofi")]
        format: Format,
        #[arg(long)]
        favorites: bool,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        rating: Option<String>,
        /// Drill into a folder (wallpaper integration only). Pass the folder path.
        #[arg(long)]
        folder: Option<String>,
        /// Apply current persisted picker state as filters. The shell side uses
        /// this so it doesn't have to re-pass all the flags every invocation.
        #[arg(long)]
        use_state: bool,
        /// Group images that share a subfolder under a single folder entry.
        #[arg(long)]
        group: bool,
    },
    /// View — render the current picker view based on persisted state.
    /// This is the one the shell wrapper calls from rofi.
    View {
        #[arg(long, default_value = "rofi")]
        format: Format,
    },
    /// List unique tags for the active integration (uses state if --integration omitted).
    Tags {
        #[arg(long, value_enum)]
        integration: Option<IntegrationArg>,
        #[arg(long, default_value = "rofi")]
        format: Format,
    },
    /// Edit user tag overrides on individual entries. Layered over native
    /// (project.json) tags at read time, so this works even for plain
    /// wallpapers that have no built-in tags.
    Tag {
        #[command(subcommand)]
        cmd: TagCmd,
    },
    /// Edit per-entry rating overrides. Layered over native ratings (from
    /// project.json) at read time. `All` clears the rating on an entry;
    /// use `rating clear` to drop the override entirely.
    Rating {
        #[command(subcommand)]
        cmd: RatingCmd,
    },
    /// Favorites management.
    Favorites {
        #[command(subcommand)]
        cmd: FavoritesCmd,
    },
    /// Picker state get/set/reset.
    State {
        #[command(subcommand)]
        cmd: StateCmd,
    },
    /// List monitors with their current wallpapers, in the requested format.
    Monitors {
        #[arg(long, value_enum)]
        integration: Option<IntegrationArg>,
        /// Entry id (image path / WE folder) being applied — included in the
        /// rofi `info` field so the shell can route the selection.
        #[arg(long)]
        target: Option<String>,
        #[arg(long, default_value = "rofi")]
        format: Format,
    },
    /// Apply an entry to a monitor.
    Apply {
        #[arg(long, value_enum)]
        integration: Option<IntegrationArg>,
        #[arg(long)]
        monitor: String,
        /// Entry id — image path or WE folder path.
        target: String,
    },
    /// Inspect or replay the per-monitor "currently applied" state. The state
    /// tracks which integration owns which monitor; `restore` re-applies
    /// everything (batching WE into a single process) so you can rehydrate
    /// the desktop from a WM/DE startup hook.
    Applied {
        #[command(subcommand)]
        cmd: AppliedCmd,
    },
    /// Daemon control.
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// Search danbooru-style image boards and download picks into
    /// `[booru].download_dir`.
    Booru {
        #[command(subcommand)]
        cmd: BooruCmd,
    },
    /// Show resolved paths and config.
    Info,
}

#[derive(Subcommand)]
pub(super) enum FavoritesCmd {
    /// List favorited entries.
    List {
        #[arg(long, value_enum)]
        integration: Option<IntegrationArg>,
        #[arg(long, default_value = "json")]
        format: Format,
    },
    /// Add a favorite. Pass either a positional `id` (single entry) or
    /// `--folder PATH` (favorites every entry directly under the folder).
    Add {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
    },
    /// Remove a favorite. `--folder` removes the favorite mark from every
    /// entry directly under the folder.
    Remove {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
    },
    /// Toggle favorite. Single-entry form prints "added" or "removed".
    /// `--folder` mode: when every entry in the folder is already a
    /// favorite, removes the mark from all of them; otherwise favorites the
    /// ones that aren't already (collective toggle).
    Toggle {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
    },
    /// Test whether an id is favorited. Exit 0 if yes, 1 if no.
    Is {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: String,
    },
}

#[derive(Subcommand)]
pub(super) enum TagCmd {
    /// Add a tag. Pass `--id ID` for a single entry, or `--folder PATH` to
    /// fan the add out across every entry directly under the folder.
    Add {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
        tag: String,
    },
    /// Remove a tag. Single-entry form cancels a prior add or hides a
    /// native tag. `--folder` mode applies the remove to every entry under
    /// the folder that currently carries the tag. Use `tag delete` to drop
    /// a tag from the entire catalog.
    Remove {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
        tag: String,
    },
    /// Replace the effective tag set with the given list.
    Set {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
        /// Tags to set. Pass repeated `--tag VALUE` for each, or none to
        /// effectively clear all native tags.
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// Drop any user overrides. `--folder` clears overrides for every
    /// entry directly under the folder.
    Clear {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
    },
    /// Print the entry's effective tags (one per line).
    Show {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
    },
    /// List the catalog of tags available for the integration. Combines
    /// native tags collected at index time with any manually-created ones.
    Available {
        #[arg(long, value_enum)]
        integration: Option<IntegrationArg>,
        #[arg(long, default_value = "rofi")]
        format: Format,
    },
    /// Register a new tag in the catalog without assigning it yet.
    Create {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        tag: String,
    },
    /// Remove a tag from the catalog entirely. By default this is a soft
    /// delete (entries that already have the tag keep it); pass `--cascade`
    /// to also strip the tag from every entry via overrides.
    Delete {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        cascade: bool,
        tag: String,
    },
}

#[derive(Subcommand)]
pub(super) enum RatingCmd {
    /// Pin a rating. `--folder` writes the same rating override onto every
    /// entry directly under the folder.
    Set {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
        #[arg(value_enum, ignore_case = true)]
        rating: crate::rating::Rating,
    },
    /// Drop the override. `--folder` clears the override on every entry
    /// directly under the folder, leaving each entry to fall back to its
    /// native rating.
    Clear {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, conflicts_with = "id")]
        folder: Option<String>,
    },
    /// Print the effective rating for this entry (empty if none).
    Show {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
pub(super) enum AppliedCmd {
    /// Print the per-monitor applied state. Default output is TSV
    /// (`<monitor>\t<integration>\t<target>`); pass `--json` for an array.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Re-apply each monitor's tracked wallpaper. WE entries collapse to a
    /// single `linux-wallpaperengine` process. Image apply hooks are skipped
    /// by default — pass `--with-hooks` to run them per monitor (matugen et
    /// al. otherwise fire N times in a row at startup).
    Restore {
        #[arg(long)]
        with_hooks: bool,
    },
    /// Drop the tracked state. Without `--monitor`, clears every monitor.
    Clear {
        #[arg(long)]
        monitor: Option<String>,
    },
}

#[derive(Subcommand)]
pub(super) enum StateCmd {
    Get {
        key: String,
    },
    Set {
        key: String,
        value: String,
    },
    Unset {
        key: String,
    },
    /// Print all state as JSON.
    Dump,
    /// Reset transient picker state (drill_path, tag_mode). Keeps picker_mode etc.
    ResetTransient,
}

#[derive(Subcommand)]
pub(super) enum DaemonCmd {
    /// Start the daemon. Without --foreground, detaches and returns.
    Start {
        #[arg(long)]
        foreground: bool,
    },
    Stop,
    Status,
}

#[derive(Subcommand)]
pub(super) enum BooruCmd {
    /// Run a tag search against a booru and cache the page as the booru
    /// index — `wallrack list --integration=booru` then renders the rows.
    Search {
        /// Site key as configured under `[booru.sites.<key>]`. Falls back
        /// to `[booru].default_site`.
        #[arg(long)]
        site: Option<String>,
        /// Tag query — passed through to the booru's search API. Use the
        /// site's own syntax (spaces = AND, `-foo` = NOT, `rating:s`, …).
        #[arg(long, default_value = "")]
        tags: String,
        /// 1-based page number. Gelbooru's 0-based `pid` is translated
        /// internally so this stays uniform across sites.
        #[arg(long, default_value_t = 1)]
        page: u32,
        /// Results per page. Most sites cap this at ~100.
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long, default_value = "json")]
        format: Format,
        /// Skip pre-downloading preview thumbs. Default is to fetch them so
        /// picker formats have an icon to render — set this for plain JSON
        /// output where the round-trips are pure waste.
        #[arg(long)]
        no_thumbs: bool,
    },
    /// Download a post from the cached search results into
    /// `[booru].download_dir`. The post id is the bare numeric id from the
    /// booru (e.g. `123456`); pass `--site` to disambiguate when the cache
    /// mixes sites.
    Download {
        /// Site key — required only if the cached index mixes sites.
        #[arg(long)]
        site: Option<String>,
        /// Post id, or the full `site:id` slug emitted by `search`.
        id: String,
    },
    /// List configured + built-in sites.
    Sites {
        #[arg(long, default_value = "json")]
        format: Format,
    },
    /// Print the currently active booru site key — the value the picker
    /// would use for the next search. Resolution order: state
    /// (`booru_site`) → config (`booru.default_site`) → first configured.
    CurrentSite,
}

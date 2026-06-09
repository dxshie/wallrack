use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

use crate::config::Config;

// ───── integration arg ───────────────────────────────────────────────────────

/// CLI surface for selecting an integration. The string values match the
/// on-disk integration keys used everywhere else (cache dirs, state, the
/// picker shell), so we can hand `as_str()` straight to `integrations::by_name`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum IntegrationArg {
    /// Plain images from `wallpaper.dirs` in config.toml.
    #[value(name = "wallpaper")]
    Wallpaper,
    /// Images extracted from Wallpaper Engine workshop projects.
    #[value(name = "we_image")]
    WeImage,
    /// Live Wallpaper Engine projects (linux-wallpaperengine).
    #[value(name = "we")]
    We,
}

impl IntegrationArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Wallpaper => "wallpaper",
            Self::WeImage => "we_image",
            Self::We => "we",
        }
    }
}

/// Like [`IntegrationArg`] but with an extra `all` value for `wallrack index`,
/// which can rebuild every integration in one go.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum IndexTarget {
    /// Rebuild every integration's index.
    All,
    #[value(name = "wallpaper")]
    Wallpaper,
    #[value(name = "we_image")]
    WeImage,
    #[value(name = "we")]
    We,
}

impl IndexTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Wallpaper => "wallpaper",
            Self::WeImage => "we_image",
            Self::We => "we",
        }
    }
}

// ───── terminal color helpers ────────────────────────────────────────────────

struct C {
    bold: &'static str,
    green: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    red: &'static str,
    dim: &'static str,
    reset: &'static str,
}

impl C {
    fn stdout() -> Self {
        Self::for_tty(io::stdout().is_terminal())
    }
    fn stderr() -> Self {
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

fn make_clap_styles() -> clap::builder::Styles {
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

// ───── notify-send helpers ────────────────────────────────────────────────────

const NOTIFY_REPLACE_ID: &str = "9991";

fn is_rofi_context() -> bool {
    std::env::var("ROFI_RETV").is_ok()
}

fn notify_send(body: &str, expire_ms: u32) {
    let _ = Command::new("notify-send")
        .arg(format!("--replace-id={NOTIFY_REPLACE_ID}"))
        .arg(format!("--expire-time={expire_ms}"))
        .arg("wallrack index")
        .arg(body)
        .status();
}
use crate::daemon::Daemon;
use crate::entry::{Entry, Index};
use crate::favorites::Favorites;
use crate::integrations::{self, Integration, wallpaper_engine};
use crate::output::{Format, Row, ViewHints, write_rows};
use crate::paths::Paths;
use crate::state::{self, State};

#[derive(Parser)]
#[command(name = "wallrack", version, about = "Modular wallpaper manager")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
    /// Daemon control.
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// Show resolved paths and config.
    Info,
}

#[derive(Subcommand)]
enum FavoritesCmd {
    /// List favorited entries.
    List {
        #[arg(long, value_enum)]
        integration: Option<IntegrationArg>,
        #[arg(long, default_value = "json")]
        format: Format,
    },
    Add {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: String,
    },
    Remove {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: String,
    },
    /// Toggle favorite. Prints "added" or "removed".
    Toggle {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: String,
    },
    /// Test whether an id is favorited. Exit 0 if yes, 1 if no.
    Is {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        id: String,
    },
}

#[derive(Subcommand)]
enum TagCmd {
    /// Add a tag to the effective set for this entry (and to the catalog).
    Add {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
        tag: String,
    },
    /// Remove a tag from THIS entry (cancels a prior add or hides a native
    /// tag). Use `tag delete` to drop a tag from every entry at once.
    Remove {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
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
    /// Drop any user overrides for this entry; falls back to native tags.
    Clear {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
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
enum RatingCmd {
    /// Pin a rating on this entry.
    Set {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
        #[arg(value_enum, ignore_case = true)]
        rating: crate::rating::Rating,
    },
    /// Drop the override; the entry falls back to its native rating.
    Clear {
        #[arg(long, value_enum)]
        integration: IntegrationArg,
        #[arg(long)]
        id: String,
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
enum StateCmd {
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
enum DaemonCmd {
    /// Start the daemon. Without --foreground, detaches and returns.
    Start {
        #[arg(long)]
        foreground: bool,
    },
    Stop,
    Status,
}

pub fn run() -> Result<ExitCode> {
    let matches = Cli::command().styles(make_clap_styles()).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let paths = Paths::discover()?;
    let config = Config::load(&paths)?;

    match cli.cmd {
        Cmd::Index { integration } => cmd_index(&paths, &config, integration.as_str()),
        Cmd::List {
            integration,
            format,
            favorites,
            tag,
            rating,
            folder,
            use_state,
            group,
        } => cmd_list(
            &paths,
            integration.as_str(),
            format,
            favorites,
            tag,
            rating,
            folder,
            use_state,
            group,
        ),
        Cmd::View { format } => cmd_view(&paths, format),
        Cmd::Tags {
            integration,
            format,
        } => cmd_tags(&paths, integration.map(|i| i.as_str()), format),
        Cmd::Tag { cmd } => cmd_tag(&paths, cmd),
        Cmd::Rating { cmd } => cmd_rating(&paths, cmd),
        Cmd::Favorites { cmd } => cmd_favorites(&paths, cmd),
        Cmd::State { cmd } => cmd_state(&paths, cmd),
        Cmd::Monitors {
            integration,
            target,
            format,
        } => cmd_monitors(
            &paths,
            &config,
            integration.map(|i| i.as_str()),
            target.as_deref(),
            format,
        ),
        Cmd::Apply {
            integration,
            monitor,
            target,
        } => cmd_apply(
            &paths,
            &config,
            integration.map(|i| i.as_str()),
            &monitor,
            &target,
        ),
        Cmd::Daemon { cmd } => cmd_daemon(&paths, &config, cmd),
        Cmd::Info => cmd_info(&paths, &config),
    }
}

// ───── index ─────────────────────────────────────────────────────────────────

fn cmd_index(paths: &Paths, config: &Config, integration: &str) -> Result<ExitCode> {
    let targets: Vec<Box<dyn Integration>> = if integration == "all" {
        integrations::all()
    } else {
        vec![integrations::by_name(integration)?]
    };

    let in_rofi = is_rofi_context();
    let c = C::stderr();
    let multi = targets.len() > 1;
    let mut total = 0usize;

    let catalog_path = paths.tag_catalog_file();
    let mut catalog = crate::tags::TagCatalog::load(&catalog_path)?;
    let mut catalog_dirty = false;

    for integ in &targets {
        if in_rofi {
            notify_send(&format!("Indexing {}…", integ.name()), 3000);
        }
        let started = std::time::Instant::now();
        match integ.index(paths, config) {
            Ok(idx) => {
                let n = idx.entries.len();
                total += n;
                let elapsed = started.elapsed().as_secs_f32();
                eprintln!(
                    "wallrack: {}{}{} indexed {}{}{} entries in {:.2}s",
                    c.yellow,
                    integ.name(),
                    c.reset,
                    c.green,
                    n,
                    c.reset,
                    elapsed,
                );
                // Pull native tags into the catalog so the picker can suggest
                // them without re-reading the whole index. Manually-created
                // catalog entries persist because we union, never replace.
                let before = catalog.list(integ.name()).len();
                catalog.extend(
                    integ.name(),
                    idx.entries.iter().flat_map(|e| e.tags.iter().cloned()),
                );
                if catalog.list(integ.name()).len() != before {
                    catalog_dirty = true;
                }
                if in_rofi && multi {
                    notify_send(
                        &format!("{}: {} entries ({:.1}s)", integ.name(), n, elapsed),
                        0,
                    );
                }
            }
            Err(err) => {
                eprintln!(
                    "wallrack: {}{}{} index failed: {err:#}",
                    c.red,
                    integ.name(),
                    c.reset
                );
                if in_rofi {
                    notify_send(&format!("{}: failed — {err}", integ.name()), 5000);
                }
            }
        }
    }

    if catalog_dirty {
        catalog.save(&catalog_path)
            .with_context(|| format!("save tag catalog {}", catalog_path.display()))?;
    }

    if in_rofi {
        let msg = if multi {
            format!("Done — {} total entries", total)
        } else {
            format!("Done — {} entries indexed", total)
        };
        notify_send(&msg, 4000);
    }

    Ok(ExitCode::SUCCESS)
}

// ───── list ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn cmd_list(
    paths: &Paths,
    integration: &str,
    format: Format,
    favorites_only: bool,
    tag: Option<String>,
    rating: Option<String>,
    folder: Option<String>,
    use_state: bool,
    group: bool,
) -> Result<ExitCode> {
    let (integration, favorites_only, tag, rating, folder, group) = if use_state {
        // Pull filter context from persisted picker state.
        let state = State::load(&paths.state_file())?;
        let picker_mode = state
            .get_or(state::keys::PICKER_MODE, "wallpaper")
            .to_string();
        let view_mode = state.get_or(state::keys::VIEW_MODE, "all").to_string();
        let drill = state.get_or(state::keys::DRILL_PATH, "").to_string();
        let tag_filter = state.get_or(state::keys::TAG_FILTER, "").to_string();
        let rating = state.get_or(state::keys::RATING, "").to_string();
        let group = drill.is_empty(); // group at top level, flat inside a folder
        (
            picker_mode,
            view_mode == "favorites",
            if tag_filter.is_empty() {
                None
            } else {
                Some(tag_filter)
            },
            if rating.is_empty() || rating == "All" {
                None
            } else {
                Some(rating)
            },
            if drill.is_empty() { None } else { Some(drill) },
            group,
        )
    } else {
        (
            integration.to_string(),
            favorites_only,
            tag,
            rating,
            folder,
            group,
        )
    };

    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::load(&paths.favorites_file())?;

    let filtered = filter_entries(
        &index,
        &favorites,
        favorites_only,
        tag.as_deref(),
        rating.as_deref(),
        folder.as_deref(),
    );

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    if filtered.is_empty() && folder.is_none() {
        emit_empty_view(&mut out, &integration, favorites_only, tag.as_deref(), format)?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(folder_path) = folder.as_deref() {
        emit_drill_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            folder_path,
            favorites_only,
            tag.as_deref(),
            format,
        )?;
    } else if group && integ.supports_drill() {
        emit_grouped_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag.as_deref(),
            format,
        )?;
    } else {
        emit_flat(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag.as_deref(),
            format,
        )?;
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

fn filter_entries<'a>(
    index: &'a Index,
    favorites: &Favorites,
    favorites_only: bool,
    tag: Option<&str>,
    rating: Option<&str>,
    folder: Option<&str>,
) -> Vec<&'a Entry> {
    index
        .entries
        .iter()
        .filter(|e| {
            if favorites_only && !favorites.is_favorite(&e.integration, &e.id) {
                return false;
            }
            if let Some(t) = tag {
                if !e.tags.iter().any(|x| x == t) {
                    return false;
                }
            }
            if let Some(r) = rating {
                if !r.is_empty() && r != "All" && e.rating != r {
                    return false;
                }
            }
            if let Some(f) = folder {
                // Match images that live directly inside `f` (trailing slash trimmed).
                let want = f.trim_end_matches('/');
                let parent = e
                    .source
                    .parent()
                    .map(|p| p.to_string_lossy().trim_end_matches('/').to_string())
                    .unwrap_or_default();
                if parent != want {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn emit_flat<W: Write>(
    w: &mut W,
    entries: &[&Entry],
    favorites: &Favorites,
    integration: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    // Image-based entries carry their id explicitly so the shell doesn't
    // have to recover it by string-splitting the display line — file paths
    // like "foo - bar.jpg" would break that on the last " - ". The `we`
    // integration uses folder paths (no " - " in workshop ids) so the
    // string-split fallback is safe and we don't override its info.
    let is_image = integration == "wallpaper" || integration == "we_image";
    let rows: Vec<Row<'_>> = entries
        .iter()
        .map(|e| Row::Entry {
            entry: e,
            favorite: favorites.is_favorite(&e.integration, &e.id),
            label: None,
            info: if is_image {
                Some(format!("image:{}", e.id))
            } else {
                None
            },
        })
        .collect();
    write_rows(
        w,
        &rows,
        &view_hints_for(integration, None, favorites_only, tag_filter),
        format,
    )
}

fn emit_drill_view<W: Write>(
    w: &mut W,
    entries: &[&Entry],
    favorites: &Favorites,
    integration: &str,
    folder_path: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    let mut rows: Vec<Row<'_>> = Vec::with_capacity(entries.len() + 1);
    rows.push(Row::Control {
        label: "← Back".to_string(),
        info: "back:".to_string(),
        icon: None,
    });
    for e in entries {
        let label = e
            .source
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| e.title.clone());
        rows.push(Row::Entry {
            entry: e,
            favorite: favorites.is_favorite(&e.integration, &e.id),
            label: Some(label),
            info: Some(format!("image:{}", e.id)),
        });
    }
    let mut hints = view_hints_for(integration, Some(folder_path), favorites_only, tag_filter);
    hints.message = "Alt+3 fav | Alt+2 tag | select ← Back to return".to_string();
    write_rows(w, &rows, &hints, format)
}

fn emit_grouped_view<W: Write>(
    w: &mut W,
    entries: &[&Entry],
    favorites: &Favorites,
    integration: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    use std::collections::BTreeSet;
    let mut rows: Vec<Row<'_>> = Vec::new();
    let mut seen_folders: BTreeSet<String> = BTreeSet::new();

    for e in entries {
        if e.subfolder.is_empty() {
            // Root-level: emit as individual entry. `image:<id>` info makes
            // the shell route to the monitor picker without parsing the
            // display line — paths containing " - " would otherwise be
            // mis-split.
            rows.push(Row::Entry {
                entry: e,
                favorite: favorites.is_favorite(&e.integration, &e.id),
                label: None,
                info: Some(format!("image:{}", e.id)),
            });
        } else {
            // Nested: emit one entry per (workshop_id, subfolder).
            let key = format!(
                "{}\u{1c}{}",
                e.workshop_id.clone().unwrap_or_default(),
                e.subfolder
            );
            if !seen_folders.insert(key) {
                continue;
            }
            let folder_path = e
                .source
                .parent()
                .map(|p| format!("{}/", p.to_string_lossy()))
                .unwrap_or_default();
            rows.push(Row::Entry {
                entry: e,
                favorite: false, // folders aren't favoritable
                label: Some(format!("{} - {}", e.title, e.subfolder)),
                info: Some(format!("folder:{folder_path}")),
            });
        }
    }
    write_rows(
        w,
        &rows,
        &view_hints_for(integration, None, favorites_only, tag_filter),
        format,
    )
}

/// Render a placeholder row when the current view would otherwise produce
/// zero entries. Rofi exits as soon as the script writes no rows, which
/// closes the picker abruptly — typically right after Alt+1 lands on an
/// integration that's unindexed or has an empty config. This keeps rofi
/// open and steers the user toward the keys that fix it.
fn emit_empty_view<W: Write>(
    w: &mut W,
    integration: &str,
    favorites_only: bool,
    tag_filter: Option<&str>,
    format: Format,
) -> Result<()> {
    let label = integrations::by_name(integration)
        .ok()
        .map(|i| i.label().to_string())
        .unwrap_or_else(|| integration.to_string());
    let index_empty = !favorites_only && tag_filter.map(|t| t.is_empty()).unwrap_or(true);
    let reason = if favorites_only {
        format!("No favorited {label} yet — Alt+3 on an entry to favorite it")
    } else if tag_filter.map(|t| !t.is_empty()).unwrap_or(false) {
        format!("No {label} match the current tag filter — Alt+2 to clear")
    } else {
        format!("No {label} indexed")
    };
    // Only suggest config edits when the *index itself* is empty — for an
    // empty favorites or tag-filter view, the index might be fine; the
    // filter is just too narrow.
    let hint = if index_empty {
        match integration {
            "wallpaper" => " — set `wallpaper.dirs` in config.toml then press Alt+0",
            "we_image" | "we" => " — check `workshop_dir` in config.toml then press Alt+0",
            _ => " — press Alt+0 to refresh or Alt+1 to switch mode",
        }
    } else {
        ""
    };
    let row = Row::Control {
        label: format!("{reason}{hint}"),
        info: "noop:empty".to_string(),
        icon: None,
    };
    let hints = view_hints_for(integration, None, favorites_only, tag_filter);
    write_rows(w, &[row], &hints, format)
}

fn view_hints_for(
    integration: &str,
    drill: Option<&str>,
    favorites_only: bool,
    tag_filter: Option<&str>,
) -> ViewHints {
    let base = match drill {
        Some(d) => folder_label(d),
        None => integrations::by_name(integration)
            .map(|i| i.label().to_string())
            .unwrap_or_else(|_| "Wallpapers".to_string()),
    };
    let mut prompt = if favorites_only {
        format!("★ {base}")
    } else {
        base
    };
    if let Some(t) = tag_filter {
        if !t.is_empty() {
            prompt = format!("{prompt} #{t}");
        }
    }
    ViewHints {
        prompt,
        message: "Alt+1 mode | Alt+2 tag | Alt+3 fav | Alt+4 view | Alt+5 edit tags | Alt+6 rating | Alt+0 refresh".to_string(),
        use_hot_keys: true,
    }
}

fn folder_label(folder_path: &str) -> String {
    folder_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(folder_path)
        .to_string()
}

// ───── view (state-driven) ───────────────────────────────────────────────────

fn cmd_view(paths: &Paths, format: Format) -> Result<ExitCode> {
    let state = State::load(&paths.state_file())?;
    let integration = state
        .get_or(state::keys::PICKER_MODE, "wallpaper")
        .to_string();
    let view_mode = state.get_or(state::keys::VIEW_MODE, "all").to_string();
    let drill = state.get_or(state::keys::DRILL_PATH, "").to_string();
    let tag_filter = state.get_or(state::keys::TAG_FILTER, "").to_string();
    let rating = state.get_or(state::keys::RATING, "").to_string();
    let tag_mode = state.get_or(state::keys::TAG_MODE, "").to_string();
    let tag_edit_target = state.get_or(state::keys::TAG_EDIT_TARGET, "").to_string();
    let tag_add_mode = state.get_or(state::keys::TAG_ADD_MODE, "").to_string();

    // The tag-editor sub-views are state-driven so wrappers don't have to
    // know how to render them. Order matches the rofi script's dispatch:
    // add-mode wins over edit-target, which wins over tag-filter selection.
    if tag_add_mode == "on" {
        return cmd_add_tag_view(paths, &integration, &tag_edit_target, format);
    }
    if !tag_edit_target.is_empty() {
        return cmd_tag_editor_view(paths, &integration, &tag_edit_target, format);
    }

    // Tag selection view short-circuits everything else.
    if tag_mode == "selecting" {
        return cmd_tags(paths, Some(&integration), format);
    }

    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::load(&paths.favorites_file())?;

    let favorites_only = view_mode == "favorites";
    let tag = if tag_filter.is_empty() {
        None
    } else {
        Some(tag_filter.as_str())
    };
    let rating_opt = if rating.is_empty() || rating == "All" {
        None
    } else {
        Some(rating.as_str())
    };
    let folder_opt = if drill.is_empty() {
        None
    } else {
        Some(drill.as_str())
    };

    let filtered = filter_entries(
        &index,
        &favorites,
        favorites_only,
        tag,
        rating_opt,
        folder_opt,
    );

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    // Empty top-level view → render a placeholder row so rofi doesn't exit.
    // The drill view always carries a "← Back" row so it can stand on its own.
    if filtered.is_empty() && folder_opt.is_none() {
        emit_empty_view(&mut out, &integration, favorites_only, tag, format)?;
        out.flush()?;
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(folder_path) = folder_opt {
        emit_drill_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            folder_path,
            favorites_only,
            tag,
            format,
        )?;
    } else if drill.is_empty() && integ.supports_drill() && !favorites_only {
        // Grouping collapses subfolders into folder rows, which is wrong for
        // the favorites view: a favorite is an individual image and Alt+3 on
        // a folder row can't recover the real entry id.
        emit_grouped_view(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag,
            format,
        )?;
    } else {
        emit_flat(
            &mut out,
            &filtered,
            &favorites,
            &integration,
            favorites_only,
            tag,
            format,
        )?;
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

// ───── tag editor sub-views (state-driven) ───────────────────────────────────

/// Render the per-entry tag editor. Rows: a Back row, an Add row, and one
/// row per tag currently on the entry — selecting a tag row removes it.
fn cmd_tag_editor_view(
    paths: &Paths,
    integration: &str,
    target: &str,
    format: Format,
) -> Result<ExitCode> {
    let integ = integrations::by_name(integration)?;
    let idx = integ.read_index(paths)?;
    let tags: Vec<String> = idx
        .entries
        .iter()
        .find(|e| e.id == target)
        .map(|e| e.tags.clone())
        .unwrap_or_default();
    let label = target.rsplit('/').next().unwrap_or(target).to_string();

    let mut rows: Vec<Row<'_>> = Vec::with_capacity(tags.len() + 2);
    rows.push(Row::Control {
        label: "← Back".to_string(),
        info: "tagedit:back".to_string(),
        icon: None,
    });
    rows.push(Row::Control {
        label: "+ Add tag…".to_string(),
        info: "tagedit:add".to_string(),
        icon: None,
    });
    for t in &tags {
        if t.is_empty() {
            continue;
        }
        rows.push(Row::Control {
            label: t.clone(),
            info: format!("tagedit:remove:{t}"),
            icon: None,
        });
    }
    let hints = ViewHints {
        prompt: format!("Tags: {label}"),
        message: "Enter to remove tag | \"+ Add\" prompts for a new tag | ← Back".to_string(),
        use_hot_keys: true,
    };
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    write_rows(&mut out, &rows, &hints, format)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Render the add-tag prompt. Rows: a Cancel row + every catalog tag for
/// the active integration. Wrappers that support free-form input let the
/// user type a brand-new tag too.
fn cmd_add_tag_view(
    paths: &Paths,
    integration: &str,
    target: &str,
    format: Format,
) -> Result<ExitCode> {
    let catalog = crate::tags::TagCatalog::load(&paths.tag_catalog_file())?;
    let tags = catalog.list(integration);
    let label = target.rsplit('/').next().unwrap_or(target).to_string();

    let mut rows: Vec<Row<'_>> = Vec::with_capacity(tags.len() + 1);
    rows.push(Row::Control {
        label: "← Cancel".to_string(),
        info: "tagedit:cancel".to_string(),
        icon: None,
    });
    for t in &tags {
        if t.is_empty() {
            continue;
        }
        rows.push(Row::Control {
            label: t.clone(),
            // The rofi script treats a non-`tagedit:*` info as "user picked a
            // catalog tag to add" — same convention here for any wrapper.
            info: format!("tagedit:pick:{t}"),
            icon: None,
        });
    }
    let hints = ViewHints {
        prompt: format!("Add tag to {label}"),
        message: "Pick a known tag or type a new one — Enter to add, Esc to cancel".to_string(),
        use_hot_keys: true,
    };
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    write_rows(&mut out, &rows, &hints, format)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

// ───── tags ──────────────────────────────────────────────────────────────────

fn cmd_tags(paths: &Paths, integration: Option<&str>, format: Format) -> Result<ExitCode> {
    let integration = match integration {
        Some(s) => s.to_string(),
        None => {
            let state = State::load(&paths.state_file())?;
            state
                .get_or(state::keys::PICKER_MODE, "wallpaper")
                .to_string()
        }
    };
    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;

    use std::collections::BTreeMap;
    // Map each tag to the first entry we see whose thumbnail exists on disk —
    // that gives rofi something to render next to the tag label. Entries
    // without a usable thumb still contribute the tag itself.
    let mut tag_thumb: BTreeMap<&str, Option<&std::path::Path>> = BTreeMap::new();
    for e in &index.entries {
        let thumb: Option<&std::path::Path> = if e.thumb.as_os_str().is_empty() {
            None
        } else {
            Some(e.thumb.as_path())
        };
        for t in &e.tags {
            if t.is_empty() {
                continue;
            }
            let slot = tag_thumb.entry(t.as_str()).or_insert(None);
            if slot.is_none() {
                if let Some(p) = thumb {
                    if p.exists() {
                        *slot = Some(p);
                    }
                }
            }
        }
    }

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    match format {
        Format::Json => {
            let list: Vec<&str> = tag_thumb.keys().copied().collect();
            serde_json::to_writer(&mut out, &list)?;
        }
        Format::Rofi | Format::Walker | Format::Wofi | Format::Raffi => {
            // Header + "All tags" reset row + one row per tag.
            let mut rows: Vec<Row<'_>> = Vec::new();
            rows.push(Row::Control {
                label: "All tags".to_string(),
                info: "tag:".to_string(),
                icon: None,
            });
            for (t, thumb) in &tag_thumb {
                rows.push(Row::Control {
                    label: t.to_string(),
                    info: format!("tag:{t}"),
                    icon: thumb.map(|p| p.to_path_buf()),
                });
            }
            let hints = ViewHints {
                prompt: "Filter by Tag".to_string(),
                message: "Select a tag — Alt+2 to cancel".to_string(),
                use_hot_keys: true,
            };
            write_rows(&mut out, &rows, &hints, format)?;
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

// ───── tag overrides ─────────────────────────────────────────────────────────

fn cmd_tag(paths: &Paths, cmd: TagCmd) -> Result<ExitCode> {
    let tags_path = paths.tags_file();
    let catalog_path = paths.tag_catalog_file();
    let mut overrides = crate::tags::TagOverrides::load(&tags_path)?;
    match cmd {
        TagCmd::Add { integration, id, tag } => {
            overrides.add(integration.as_str(), &id, &tag);
            overrides.save(&tags_path)?;
            // Newly-added tags should be immediately suggestable in the
            // picker, so reflect them in the catalog right away rather than
            // waiting for the next re-index.
            let mut catalog = crate::tags::TagCatalog::load(&catalog_path)?;
            if catalog.add(integration.as_str(), &tag) {
                catalog.save(&catalog_path)?;
            }
        }
        TagCmd::Remove { integration, id, tag } => {
            overrides.remove(integration.as_str(), &id, &tag);
            overrides.save(&tags_path)?;
        }
        TagCmd::Set { integration, id, tags } => {
            // Need the native tag set to compute a minimal override that
            // survives index regeneration. If the entry isn't in the index
            // yet, fall back to "no native tags" — the override just becomes
            // pure additive.
            let integ = integrations::by_name(integration.as_str())?;
            let native: Vec<String> = match integ.read_index(paths) {
                Ok(idx) => {
                    // read_index already applies overrides; recover the
                    // native tags by stripping this entry's current
                    // overrides off the effective set we got back.
                    let effective = idx.entries.iter().find(|e| e.id == id)
                        .map(|e| e.tags.clone()).unwrap_or_default();
                    let prior = overrides.get(integration.as_str(), &id).cloned().unwrap_or_default();
                    // native = (effective ∪ prior.removed) \ prior.added
                    let mut native: std::collections::BTreeSet<String> = effective.into_iter().collect();
                    native.extend(prior.removed.iter().cloned());
                    for added in &prior.added { native.remove(added); }
                    native.into_iter().collect()
                }
                Err(_) => Vec::new(),
            };
            overrides.set(integration.as_str(), &id, &tags, &native);
            overrides.save(&tags_path)?;
            let mut catalog = crate::tags::TagCatalog::load(&catalog_path)?;
            catalog.extend(integration.as_str(), tags.iter().cloned());
            catalog.save(&catalog_path)?;
        }
        TagCmd::Clear { integration, id } => {
            overrides.clear(integration.as_str(), &id);
            overrides.save(&tags_path)?;
        }
        TagCmd::Show { integration, id } => {
            let integ = integrations::by_name(integration.as_str())?;
            let idx = integ.read_index(paths)?;
            if let Some(entry) = idx.entries.iter().find(|e| e.id == id) {
                for t in &entry.tags {
                    println!("{t}");
                }
            } else {
                return Err(anyhow!("entry not in index: {id}"));
            }
        }
        TagCmd::Available { integration, format } => {
            let integration: String = match integration {
                Some(s) => s.as_str().to_string(),
                None => {
                    let state = State::load(&paths.state_file())?;
                    state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string()
                }
            };
            let catalog = crate::tags::TagCatalog::load(&catalog_path)?;
            let tags = catalog.list(&integration);
            match format {
                Format::Json => {
                    let stdout = io::stdout().lock();
                    serde_json::to_writer(stdout, &tags)?;
                }
                // Plain-text formats. The picker scripts feed this list as
                // candidate input to their search box; no icons or routing
                // info is needed here.
                Format::Rofi | Format::Walker | Format::Wofi | Format::Raffi => {
                    for t in tags {
                        println!("{t}");
                    }
                }
            }
        }
        TagCmd::Create { integration, tag } => {
            let mut catalog = crate::tags::TagCatalog::load(&catalog_path)?;
            if catalog.add(integration.as_str(), &tag) {
                catalog.save(&catalog_path)?;
            }
        }
        TagCmd::Delete { integration, cascade, tag } => {
            let mut catalog = crate::tags::TagCatalog::load(&catalog_path)?;
            catalog.remove(integration.as_str(), &tag);
            catalog.save(&catalog_path)?;
            if cascade {
                // Hide the tag on every entry that currently carries it —
                // including native tags from project.json — by writing a
                // `removed` override per affected entry.
                let integ = integrations::by_name(integration.as_str())?;
                if let Ok(idx) = integ.read_index(paths) {
                    let mut touched = false;
                    for entry in &idx.entries {
                        if entry.tags.iter().any(|t| t == &tag) {
                            overrides.remove(integration.as_str(), &entry.id, &tag);
                            touched = true;
                        }
                    }
                    if touched {
                        overrides.save(&tags_path)?;
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ───── rating overrides ──────────────────────────────────────────────────────

fn cmd_rating(paths: &Paths, cmd: RatingCmd) -> Result<ExitCode> {
    let path = paths.rating_overrides_file();
    let mut overrides = crate::rating::RatingOverrides::load(&path)?;
    match cmd {
        RatingCmd::Set { integration, id, rating } => {
            overrides.set(integration.as_str(), &id, rating);
            overrides.save(&path)?;
        }
        RatingCmd::Clear { integration, id } => {
            overrides.clear(integration.as_str(), &id);
            overrides.save(&path)?;
        }
        RatingCmd::Show { integration, id } => {
            let integ = integrations::by_name(integration.as_str())?;
            let idx = integ.read_index(paths)?;
            if let Some(entry) = idx.entries.iter().find(|e| e.id == id) {
                println!("{}", entry.rating);
            } else {
                return Err(anyhow!("entry not in index: {id}"));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ───── favorites ─────────────────────────────────────────────────────────────

fn cmd_favorites(paths: &Paths, cmd: FavoritesCmd) -> Result<ExitCode> {
    let fav_path = paths.favorites_file();
    let mut favorites = Favorites::load(&fav_path)?;
    match cmd {
        FavoritesCmd::List {
            integration,
            format,
        } => {
            let integration: String = match integration {
                Some(s) => s.as_str().to_string(),
                None => {
                    let state = State::load(&paths.state_file())?;
                    state
                        .get_or(state::keys::PICKER_MODE, "wallpaper")
                        .to_string()
                }
            };
            let ids = favorites.list(&integration);
            match format {
                Format::Json => {
                    let stdout = io::stdout().lock();
                    serde_json::to_writer(stdout, &ids)?;
                }
                // Plain id-per-line — every picker can consume that.
                Format::Rofi | Format::Walker | Format::Wofi | Format::Raffi => {
                    for id in ids {
                        println!("{id}");
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Add { integration, id } => {
            favorites.add(integration.as_str(), &id);
            favorites.save(&fav_path)?;
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Remove { integration, id } => {
            favorites.remove(integration.as_str(), &id);
            favorites.save(&fav_path)?;
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Toggle { integration, id } => {
            let now_fav = favorites.toggle(integration.as_str(), &id);
            favorites.save(&fav_path)?;
            println!("{}", if now_fav { "added" } else { "removed" });
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Is { integration, id } => {
            Ok(if favorites.is_favorite(integration.as_str(), &id) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

// ───── state ─────────────────────────────────────────────────────────────────

fn cmd_state(paths: &Paths, cmd: StateCmd) -> Result<ExitCode> {
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
            state.save(&state_path)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ───── monitors ──────────────────────────────────────────────────────────────

fn cmd_monitors(
    paths: &Paths,
    config: &Config,
    integration: Option<&str>,
    target: Option<&str>,
    format: Format,
) -> Result<ExitCode> {
    let integration = match integration {
        Some(s) => s.to_string(),
        None => {
            let state = State::load(&paths.state_file())?;
            state
                .get_or(state::keys::PICKER_MODE, "wallpaper")
                .to_string()
        }
    };

    let integ = integrations::by_name(&integration)?;
    let merged = integ.merged_backend(config);
    let monitors = integrations::backend::run_monitors(&merged)
        .with_context(|| format!("list monitors for {integration}"))?;
    let thumbs = current_thumbs(&integration, paths, config);

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    match format {
        Format::Json => {
            let list: Vec<_> = monitors
                .iter()
                .map(|m| {
                    let icon = thumbs.get(m);
                    serde_json::json!({ "name": m, "current_icon": icon })
                })
                .collect();
            serde_json::to_writer(&mut out, &list)?;
        }
        Format::Rofi | Format::Walker | Format::Wofi | Format::Raffi => {
            // One row per monitor. The `info` payload carries the entry id
            // (image path or WE folder) so the wrapper can route the apply
            // call after the user picks a monitor.
            let rows: Vec<Row<'_>> = monitors
                .iter()
                .map(|m| Row::Control {
                    label: m.clone(),
                    info: target.unwrap_or_default().to_string(),
                    icon: thumbs.get(m).cloned(),
                })
                .collect();
            let hints = ViewHints {
                prompt: "Monitor".to_string(),
                message: String::new(),
                use_hot_keys: false,
            };
            write_rows(&mut out, &rows, &hints, format)?;
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// Per-monitor current-wallpaper thumbnails. WE tracks its own state
/// (linux-wallpaperengine has no introspection), the other integrations rely
/// on the backend's optional `current_image_cmd`.
fn current_thumbs(
    integration: &str,
    paths: &Paths,
    config: &Config,
) -> std::collections::HashMap<String, PathBuf> {
    use std::collections::HashMap;
    if integration == "we" {
        let state = wallpaper_engine::read_monitor_state(paths);
        if state.is_empty() {
            return HashMap::new();
        }
        let idx_path = paths.index_file("we");
        let Ok(raw) = std::fs::read_to_string(&idx_path) else { return HashMap::new() };
        let Ok(idx) = serde_json::from_str::<Index>(&raw) else { return HashMap::new() };
        let by_workshop: HashMap<String, PathBuf> = idx
            .entries
            .into_iter()
            .filter_map(|e| e.workshop_id.map(|w| (w, e.thumb)))
            .collect();
        return state
            .into_iter()
            .filter_map(|(mon, wid)| by_workshop.get(&wid).cloned().map(|t| (mon, t)))
            .collect();
    }
    let Ok(integ) = integrations::by_name(integration) else { return HashMap::new() };
    integrations::backend::run_current_image(&integ.merged_backend(config))
}

// ───── apply ─────────────────────────────────────────────────────────────────

fn cmd_apply(
    paths: &Paths,
    config: &Config,
    integration: Option<&str>,
    monitor: &str,
    target: &str,
) -> Result<ExitCode> {
    let integration = match integration {
        Some(s) => s.to_string(),
        None => {
            let state = State::load(&paths.state_file())?;
            state
                .get_or(state::keys::PICKER_MODE, "wallpaper")
                .to_string()
        }
    };
    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let entry = index
        .entries
        .iter()
        .find(|e| e.id == target)
        .cloned()
        .ok_or_else(|| anyhow!("entry not in index: {target}"))?;
    integ.apply(&entry, monitor, paths, config)?;
    Ok(ExitCode::SUCCESS)
}

// ───── daemon ────────────────────────────────────────────────────────────────

fn cmd_daemon(paths: &Paths, config: &Config, cmd: DaemonCmd) -> Result<ExitCode> {
    let d = Daemon::new(paths);
    match cmd {
        DaemonCmd::Start { foreground } => {
            d.start(config, foreground)?;
            Ok(ExitCode::SUCCESS)
        }
        DaemonCmd::Stop => {
            d.stop()?;
            Ok(ExitCode::SUCCESS)
        }
        DaemonCmd::Status => {
            d.status()?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ───── info ──────────────────────────────────────────────────────────────────

fn cmd_info(paths: &Paths, config: &Config) -> Result<ExitCode> {
    let c = C::stdout();

    println!(
        "{}config:{} {}",
        c.bold,
        c.reset,
        paths.config_file().display()
    );
    println!(
        "{}cache:{}  {}",
        c.bold,
        c.reset,
        paths.cache_dir().display()
    );

    // Collect per-integration indexes (best-effort; missing index → 0 entries).
    let mut total_entries: usize = 0;
    let mut integration_indexes: Vec<(&'static str, Option<Index>)> = Vec::new();
    for integ in integrations::all() {
        let idx = integ.read_index(paths).ok();
        if let Some(ref i) = idx {
            total_entries += i.entries.len();
        }
        integration_indexes.push((integ.name(), idx));
    }

    println!(
        "{}index:{} {}{}{} total entries",
        c.bold, c.reset, c.green, total_entries, c.reset
    );
    println!("{}integrations:{}", c.bold, c.reset);
    for (name, idx) in &integration_indexes {
        let file = paths.index_file(name);
        match idx {
            Some(i) => println!(
                "  {}{:<12}{}  {}{:>6}{} entries  {}({}){}",
                c.cyan,
                name,
                c.reset,
                c.green,
                i.entries.len(),
                c.reset,
                c.dim,
                file.display(),
                c.reset,
            ),
            None => println!(
                "  {}{:<12}{}  {}missing{}      {}({}){}",
                c.yellow,
                name,
                c.reset,
                c.red,
                c.reset,
                c.dim,
                file.display(),
                c.reset,
            ),
        }
    }

    // Per-wallpaper-dir counts from the wallpaper integration index.
    let wp_entries: Vec<_> = integration_indexes
        .iter()
        .find(|(n, _)| *n == "wallpaper")
        .and_then(|(_, idx)| idx.as_ref())
        .map(|i| &i.entries[..])
        .unwrap_or(&[])
        .to_vec();

    println!("{}wallpaper dirs:{}", c.bold, c.reset);
    for d in config.wallpaper_dirs() {
        let count = wp_entries
            .iter()
            .filter(|e| e.source.starts_with(&d))
            .count();
        println!(
            "  {}{:>6}{} entries  {}",
            c.green,
            count,
            c.reset,
            d.display()
        );
    }
    println!(
        "{}WE image workshop dir:{} {}",
        c.bold,
        c.reset,
        config.we_image_workshop_dir().display()
    );
    println!(
        "{}WE workshop dir:{}       {}",
        c.bold,
        c.reset,
        config.we_workshop_dir().display()
    );
    Ok(ExitCode::SUCCESS)
}

use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde::Deserialize;

use crate::config::Config;

// ───── terminal color helpers ────────────────────────────────────────────────

struct C {
    bold:   &'static str,
    green:  &'static str,
    yellow: &'static str,
    cyan:   &'static str,
    red:    &'static str,
    dim:    &'static str,
    reset:  &'static str,
}

impl C {
    fn stdout() -> Self { Self::for_tty(io::stdout().is_terminal()) }
    fn stderr() -> Self { Self::for_tty(io::stderr().is_terminal()) }
    fn for_tty(on: bool) -> Self {
        if on {
            Self { bold: "\x1b[1m", green: "\x1b[32m", yellow: "\x1b[33m",
                   cyan: "\x1b[36m", red: "\x1b[31m", dim: "\x1b[2m", reset: "\x1b[0m" }
        } else {
            Self { bold: "", green: "", yellow: "", cyan: "", red: "", dim: "", reset: "" }
        }
    }
}

fn make_clap_styles() -> clap::builder::Styles {
    use clap::builder::styling::{AnsiColor, Effects, Styles};
    Styles::styled()
        .header(AnsiColor::Yellow.on_default()  | Effects::BOLD)
        .usage(AnsiColor::Green.on_default()    | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default()   | Effects::BOLD)
        .placeholder(AnsiColor::White.on_default())
        .error(AnsiColor::Red.on_default()      | Effects::BOLD)
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
        #[arg(long, default_value = "all")]
        integration: String,
    },
    /// Emit wallpapers in the requested format.
    List {
        #[arg(long, default_value = "wallpaper")]
        integration: String,
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
        #[arg(long)]
        integration: Option<String>,
        #[arg(long, default_value = "rofi")]
        format: Format,
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
        #[arg(long)]
        integration: Option<String>,
        /// Entry id (image path / WE folder) being applied — included in the
        /// rofi `info` field so the shell can route the selection.
        #[arg(long)]
        target: Option<String>,
        #[arg(long, default_value = "rofi")]
        format: Format,
    },
    /// Apply an entry to a monitor.
    Apply {
        #[arg(long)]
        integration: Option<String>,
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
        #[arg(long)]
        integration: Option<String>,
        #[arg(long, default_value = "json")]
        format: Format,
    },
    Add { #[arg(long)] integration: String, id: String },
    Remove { #[arg(long)] integration: String, id: String },
    /// Toggle favorite. Prints "added" or "removed".
    Toggle { #[arg(long)] integration: String, id: String },
    /// Test whether an id is favorited. Exit 0 if yes, 1 if no.
    Is { #[arg(long)] integration: String, id: String },
}

#[derive(Subcommand)]
enum StateCmd {
    Get { key: String },
    Set { key: String, value: String },
    Unset { key: String },
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
        Cmd::Index { integration } => cmd_index(&paths, &config, &integration),
        Cmd::List { integration, format, favorites, tag, rating, folder, use_state, group } => {
            cmd_list(&paths, &integration, format, favorites, tag, rating, folder, use_state, group)
        }
        Cmd::View { format } => cmd_view(&paths, format),
        Cmd::Tags { integration, format } => cmd_tags(&paths, integration.as_deref(), format),
        Cmd::Favorites { cmd } => cmd_favorites(&paths, cmd),
        Cmd::State { cmd } => cmd_state(&paths, cmd),
        Cmd::Monitors { integration, target, format } => cmd_monitors(&paths, integration.as_deref(), target.as_deref(), format),
        Cmd::Apply { integration, monitor, target } => cmd_apply(&paths, integration.as_deref(), &monitor, &target),
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

    for integ in &targets {
        if in_rofi {
            notify_send(&format!("Indexing {}…", integ.name()), 0);
        }
        let started = std::time::Instant::now();
        match integ.index(paths, config) {
            Ok(idx) => {
                let n = idx.entries.len();
                total += n;
                let elapsed = started.elapsed().as_secs_f32();
                eprintln!(
                    "wallrack: {}{}{} indexed {}{}{} entries in {:.2}s",
                    c.yellow, integ.name(), c.reset,
                    c.green, n, c.reset,
                    elapsed,
                );
                if in_rofi && multi {
                    notify_send(&format!("{}: {} entries ({:.1}s)", integ.name(), n, elapsed), 0);
                }
            }
            Err(err) => {
                eprintln!("wallrack: {}{}{} index failed: {err:#}", c.red, integ.name(), c.reset);
                if in_rofi {
                    notify_send(&format!("{}: failed — {err}", integ.name()), 5000);
                }
            }
        }
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
        let picker_mode = state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string();
        let view_mode = state.get_or(state::keys::VIEW_MODE, "all").to_string();
        let drill = state.get_or(state::keys::DRILL_PATH, "").to_string();
        let tag_filter = state.get_or(state::keys::TAG_FILTER, "").to_string();
        let rating = state.get_or(state::keys::RATING, "").to_string();
        let group = drill.is_empty(); // group at top level, flat inside a folder
        (
            picker_mode,
            view_mode == "favorites",
            if tag_filter.is_empty() { None } else { Some(tag_filter) },
            if rating.is_empty() || rating == "All" { None } else { Some(rating) },
            if drill.is_empty() { None } else { Some(drill) },
            group,
        )
    } else {
        (integration.to_string(), favorites_only, tag, rating, folder, group)
    };

    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::load(&paths.favorites_file())?;

    let filtered = filter_entries(&index, &favorites, favorites_only, tag.as_deref(), rating.as_deref(), folder.as_deref());

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    if let Some(folder_path) = folder.as_deref() {
        emit_drill_view(&mut out, &filtered, &favorites, &integration, folder_path, favorites_only, tag.as_deref(), format)?;
    } else if group && integration == "wallpaper" {
        emit_grouped_view(&mut out, &filtered, &favorites, &integration, favorites_only, tag.as_deref(), format)?;
    } else {
        emit_flat(&mut out, &filtered, &favorites, &integration, favorites_only, tag.as_deref(), format)?;
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
    let rows: Vec<Row<'_>> = entries
        .iter()
        .map(|e| Row::Entry {
            entry: e,
            favorite: favorites.is_favorite(&e.integration, &e.id),
            label: None,
            info: None,
        })
        .collect();
    write_rows(w, &rows, &view_hints_for(integration, None, favorites_only, tag_filter), format)
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
            // Root-level: emit as individual entry.
            rows.push(Row::Entry {
                entry: e,
                favorite: favorites.is_favorite(&e.integration, &e.id),
                label: None,
                info: None,
            });
        } else {
            // Nested: emit one entry per (workshop_id, subfolder).
            let key = format!("{}\u{1c}{}", e.workshop_id.clone().unwrap_or_default(), e.subfolder);
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
    write_rows(w, &rows, &view_hints_for(integration, None, favorites_only, tag_filter), format)
}

fn view_hints_for(
    integration: &str,
    drill: Option<&str>,
    favorites_only: bool,
    tag_filter: Option<&str>,
) -> ViewHints {
    let base = match (integration, drill) {
        (_, Some(d)) => folder_label(d),
        ("we", _) => "WE".to_string(),
        _ => "Wallpapers".to_string(),
    };
    let mut prompt = if favorites_only { format!("★ {base}") } else { base };
    if let Some(t) = tag_filter {
        if !t.is_empty() {
            prompt = format!("{prompt} #{t}");
        }
    }
    ViewHints {
        prompt,
        message: "Alt+1 mode | Alt+2 tag | Alt+3 fav | Alt+4 view | Alt+0 refresh".to_string(),
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
    let integration = state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string();
    let view_mode = state.get_or(state::keys::VIEW_MODE, "all").to_string();
    let drill = state.get_or(state::keys::DRILL_PATH, "").to_string();
    let tag_filter = state.get_or(state::keys::TAG_FILTER, "").to_string();
    let rating = state.get_or(state::keys::RATING, "").to_string();
    let tag_mode = state.get_or(state::keys::TAG_MODE, "").to_string();

    // Tag selection view short-circuits everything else.
    if tag_mode == "selecting" {
        return cmd_tags(paths, Some(&integration), format);
    }

    let integ = integrations::by_name(&integration)?;
    let index = integ.read_index(paths)?;
    let favorites = Favorites::load(&paths.favorites_file())?;

    let favorites_only = view_mode == "favorites";
    let tag = if tag_filter.is_empty() { None } else { Some(tag_filter.as_str()) };
    let rating_opt = if rating.is_empty() || rating == "All" { None } else { Some(rating.as_str()) };
    let folder_opt = if drill.is_empty() { None } else { Some(drill.as_str()) };

    let filtered = filter_entries(&index, &favorites, favorites_only, tag, rating_opt, folder_opt);

    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    if let Some(folder_path) = folder_opt {
        emit_drill_view(&mut out, &filtered, &favorites, &integration, folder_path, favorites_only, tag, format)?;
    } else if drill.is_empty() && integration == "wallpaper" && !favorites_only {
        // Grouping collapses workshop subfolders into folder rows, which is
        // wrong for the favorites view: a favorite is an individual image and
        // Alt+3 on a folder row can't recover the real entry id.
        emit_grouped_view(&mut out, &filtered, &favorites, &integration, favorites_only, tag, format)?;
    } else {
        emit_flat(&mut out, &filtered, &favorites, &integration, favorites_only, tag, format)?;
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

// ───── tags ──────────────────────────────────────────────────────────────────

fn cmd_tags(paths: &Paths, integration: Option<&str>, format: Format) -> Result<ExitCode> {
    let integration = match integration {
        Some(s) => s.to_string(),
        None => {
            let state = State::load(&paths.state_file())?;
            state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string()
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
            if t.is_empty() { continue; }
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
        Format::Rofi => {
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

// ───── favorites ─────────────────────────────────────────────────────────────

fn cmd_favorites(paths: &Paths, cmd: FavoritesCmd) -> Result<ExitCode> {
    let fav_path = paths.favorites_file();
    let mut favorites = Favorites::load(&fav_path)?;
    match cmd {
        FavoritesCmd::List { integration, format } => {
            let integration = match integration {
                Some(s) => s,
                None => {
                    let state = State::load(&paths.state_file())?;
                    state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string()
                }
            };
            let ids = favorites.list(&integration);
            match format {
                Format::Json => {
                    let stdout = io::stdout().lock();
                    serde_json::to_writer(stdout, &ids)?;
                }
                Format::Rofi => {
                    for id in ids { println!("{id}"); }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Add { integration, id } => {
            favorites.add(&integration, &id);
            favorites.save(&fav_path)?;
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Remove { integration, id } => {
            favorites.remove(&integration, &id);
            favorites.save(&fav_path)?;
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Toggle { integration, id } => {
            let now_fav = favorites.toggle(&integration, &id);
            favorites.save(&fav_path)?;
            println!("{}", if now_fav { "added" } else { "removed" });
            Ok(ExitCode::SUCCESS)
        }
        FavoritesCmd::Is { integration, id } => {
            Ok(if favorites.is_favorite(&integration, &id) {
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
            state.save(&state_path)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ───── monitors ──────────────────────────────────────────────────────────────

fn cmd_monitors(paths: &Paths, integration: Option<&str>, target: Option<&str>, format: Format) -> Result<ExitCode> {
    let integration = match integration {
        Some(s) => s.to_string(),
        None => {
            let state = State::load(&paths.state_file())?;
            state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string()
        }
    };

    let monitors = list_monitors()?;
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);

    match format {
        Format::Json => {
            let list: Vec<_> = monitors
                .iter()
                .map(|m| {
                    let icon = current_thumb_for_monitor(&integration, m, paths);
                    serde_json::json!({ "name": m, "current_icon": icon })
                })
                .collect();
            serde_json::to_writer(&mut out, &list)?;
        }
        Format::Rofi => {
            // Emit one rofi row per monitor. The `info` field carries the
            // target (image path or WE folder) so the shell can apply once
            // the user picks a monitor. Rofi expects a single \0 between the
            // display text and the metadata block; subsequent pairs are
            // separated by \x1f — a second \0 hides everything after it.
            for m in &monitors {
                let icon = current_thumb_for_monitor(&integration, m, paths);
                out.write_all(m.as_bytes())?;
                let mut wrote_meta = false;
                if let Some(icon) = icon {
                    out.write_all(&[0])?;
                    wrote_meta = true;
                    write!(out, "icon")?;
                    out.write_all(&[0x1f])?;
                    out.write_all(icon.to_string_lossy().as_bytes())?;
                }
                if let Some(t) = target {
                    out.write_all(&[if wrote_meta { 0x1f } else { 0 }])?;
                    write!(out, "info")?;
                    out.write_all(&[0x1f])?;
                    out.write_all(t.as_bytes())?;
                }
                writeln!(out)?;
            }
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Deserialize)]
struct HyprMonitor { name: String }

fn list_monitors() -> Result<Vec<String>> {
    let out = Command::new("hyprctl").arg("monitors").arg("-j").output()
        .context("hyprctl monitors -j")?;
    if !out.status.success() {
        return Err(anyhow!("hyprctl exited with {}", out.status));
    }
    let mons: Vec<HyprMonitor> = serde_json::from_slice(&out.stdout)
        .context("parse hyprctl json")?;
    Ok(mons.into_iter().map(|m| m.name).collect())
}

fn current_thumb_for_monitor(integration: &str, monitor: &str, paths: &Paths) -> Option<PathBuf> {
    match integration {
        "we" => {
            let state = wallpaper_engine::read_monitor_state(paths);
            let wid = state.get(monitor)?;
            // Look up the WE entry by workshop id and use its preview.
            let idx_path = paths.index_file("we");
            let raw = std::fs::read_to_string(&idx_path).ok()?;
            let idx: Index = serde_json::from_str(&raw).ok()?;
            idx.entries
                .into_iter()
                .find(|e| e.workshop_id.as_deref() == Some(wid.as_str()))
                .map(|e| e.thumb)
        }
        "wallpaper" => {
            // awww query lines look like
            //   `: <monitor>: <details> currently displaying: image: <path>`
            // Older swww output dropped the leading `: `, so trim it before
            // matching the monitor prefix.
            let out = Command::new("awww").arg("query").output().ok()?;
            if !out.status.success() { return None; }
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let trimmed = line.trim_start_matches(|c: char| c == ':' || c == ' ');
                let Some(rest) = trimmed.strip_prefix(&format!("{monitor}:")) else { continue };
                if let Some(img) = rest.split("image:").nth(1) {
                    return Some(PathBuf::from(img.trim()));
                }
            }
            None
        }
        _ => None,
    }
}

// ───── apply ─────────────────────────────────────────────────────────────────

fn cmd_apply(paths: &Paths, integration: Option<&str>, monitor: &str, target: &str) -> Result<ExitCode> {
    let integration = match integration {
        Some(s) => s.to_string(),
        None => {
            let state = State::load(&paths.state_file())?;
            state.get_or(state::keys::PICKER_MODE, "wallpaper").to_string()
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
    integ.apply(&entry, monitor, paths)?;
    Ok(ExitCode::SUCCESS)
}

// ───── daemon ────────────────────────────────────────────────────────────────

fn cmd_daemon(paths: &Paths, config: &Config, cmd: DaemonCmd) -> Result<ExitCode> {
    let d = Daemon::new(paths);
    match cmd {
        DaemonCmd::Start { foreground } => { d.start(config, foreground)?; Ok(ExitCode::SUCCESS) }
        DaemonCmd::Stop => { d.stop()?; Ok(ExitCode::SUCCESS) }
        DaemonCmd::Status => { d.status()?; Ok(ExitCode::SUCCESS) }
    }
}

// ───── info ──────────────────────────────────────────────────────────────────

fn cmd_info(paths: &Paths, config: &Config) -> Result<ExitCode> {
    let c = C::stdout();

    println!("{}config:{} {}", c.bold, c.reset, paths.config_file().display());
    println!("{}cache:{}  {}", c.bold, c.reset, paths.cache_dir().display());

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

    println!("{}index:{} {}{}{} total entries", c.bold, c.reset, c.green, total_entries, c.reset);
    println!("{}integrations:{}", c.bold, c.reset);
    for (name, idx) in &integration_indexes {
        let file = paths.index_file(name);
        match idx {
            Some(i) => println!(
                "  {}{:<12}{}  {}{:>6}{} entries  {}({}){}",
                c.cyan, name, c.reset,
                c.green, i.entries.len(), c.reset,
                c.dim, file.display(), c.reset,
            ),
            None => println!(
                "  {}{:<12}{}  {}missing{}      {}({}){}",
                c.yellow, name, c.reset,
                c.red, c.reset,
                c.dim, file.display(), c.reset,
            ),
        }
    }

    // Per-wallpaper-dir counts from the wallpaper integration index.
    let wp_entries: Vec<_> = integration_indexes.iter()
        .find(|(n, _)| *n == "wallpaper")
        .and_then(|(_, idx)| idx.as_ref())
        .map(|i| &i.entries[..])
        .unwrap_or(&[])
        .to_vec();

    println!("{}wallpaper dirs:{}", c.bold, c.reset);
    for d in config.wallpaper_dirs() {
        let count = wp_entries.iter()
            .filter(|e| e.source.starts_with(&d))
            .count();
        println!("  {}{:>6}{} entries  {}", c.green, count, c.reset, d.display());
    }
    if let Some(d) = config.wallpaper_steam_dir() {
        println!("{}wallpaper steam dir:{} {}", c.bold, c.reset, d.display());
    }
    println!("{}WE workshop dir:{} {}", c.bold, c.reset, config.we_workshop_dir().display());
    Ok(ExitCode::SUCCESS)
}

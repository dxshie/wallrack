//! `wallrack applied` — read / replay / clear the per-monitor applied state.

use std::io::{self, BufWriter, Write};
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::applied::Applied;
use crate::config::Config;
use crate::integrations::{self, wallpaper_engine};
use crate::paths::Paths;

use super::super::args::AppliedCmd;
use super::super::hooks::run_apply_hook;
use super::apply::resolve_entry;

pub(in crate::cli) fn run(paths: &Paths, config: &Config, cmd: AppliedCmd) -> Result<ExitCode> {
    match cmd {
        AppliedCmd::List { json } => list(paths, json),
        AppliedCmd::Restore { with_hooks } => restore(paths, config, with_hooks),
        AppliedCmd::Clear { monitor } => clear(paths, monitor.as_deref()),
    }
}

fn list(paths: &Paths, json: bool) -> Result<ExitCode> {
    let applied = Applied::open(paths.store())?;
    let all = applied.all();
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    if json {
        let list: Vec<_> = all
            .into_iter()
            .map(|(mon, e)| {
                serde_json::json!({
                    "monitor": mon,
                    "integration": e.integration,
                    "target": e.target,
                })
            })
            .collect();
        serde_json::to_writer(&mut out, &list)?;
    } else {
        // Tab-separated for easy shell consumption; text is the default for
        // this command so a WM start-up script can `awk` it directly.
        for (mon, e) in all {
            writeln!(out, "{}\t{}\t{}", mon, e.integration, e.target)?;
        }
    }
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

fn clear(paths: &Paths, monitor: Option<&str>) -> Result<ExitCode> {
    let applied = Applied::open(paths.store())?;
    match monitor {
        Some(m) => {
            applied.remove(m)?;
        }
        None => {
            for mon in applied.all().keys().cloned().collect::<Vec<_>>() {
                applied.remove(&mon)?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn restore(paths: &Paths, config: &Config, with_hooks: bool) -> Result<ExitCode> {
    let applied = Applied::open(paths.store())?;
    let all = applied.all();

    // Image integrations first — each writes to its own compositor. We
    // sidestep `Integration::apply` for image integrations here because their
    // apply path eagerly calls `wallpaper_engine::release_monitor`, which
    // would race against the batched WE launch we do at the end (each
    // release_monitor would pkill the WE process and try to spawn a partial
    // one). Instead we hit the backend command directly per monitor.
    for (mon, e) in &all {
        if e.integration == wallpaper_engine::NAME {
            continue;
        }
        if let Err(err) = restore_one(&e.integration, mon, &e.target, paths, config, with_hooks) {
            log::warn!("restore: {}/{} failed: {err:#}", e.integration, mon);
        }
    }

    // WE last, in one go: composes a single `linux-wallpaperengine` with
    // every WE-owned (monitor, workshop_id) pair.
    let we_monitors = applied.by_integration(wallpaper_engine::NAME);
    if !we_monitors.is_empty() {
        if with_hooks {
            for (mon, wid) in &we_monitors {
                run_apply_hook(
                    "pre_apply_hook",
                    &config.hooks.pre_apply_hook,
                    wid,
                    mon,
                    wallpaper_engine::NAME,
                )?;
            }
        }
        wallpaper_engine::launch_for(&we_monitors, config)
            .context("restore: launch linux-wallpaperengine")?;
        if with_hooks {
            for (mon, wid) in &we_monitors {
                run_apply_hook(
                    "post_apply_hook",
                    &config.hooks.post_apply_hook,
                    wid,
                    mon,
                    wallpaper_engine::NAME,
                )?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn restore_one(
    integration: &str,
    monitor: &str,
    target: &str,
    paths: &Paths,
    config: &Config,
    with_hooks: bool,
) -> Result<()> {
    let integ = integrations::by_name(integration)?;
    let entry = resolve_entry(integration, target, paths)?;
    if with_hooks {
        run_apply_hook(
            "pre_apply_hook",
            &config.hooks.pre_apply_hook,
            target,
            monitor,
            integration,
        )?;
    }
    // Image integrations' Integration::apply paint with their backend command
    // and call release_monitor — at restore time there's no live WE yet, so
    // release_monitor is a cheap no-op. Going through the trait keeps the
    // single source of truth for the command template.
    integ.apply(&entry, monitor, paths, config)?;
    if with_hooks {
        run_apply_hook(
            "post_apply_hook",
            &config.hooks.post_apply_hook,
            target,
            monitor,
            integration,
        )?;
    }
    Ok(())
}

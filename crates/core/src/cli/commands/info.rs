use std::process::ExitCode;

use anyhow::Result;

use crate::config::Config;
use crate::entry::Index;
use crate::integrations;
use crate::paths::Paths;

use super::super::style::C;

pub(in crate::cli) fn run(paths: &Paths, config: &Config) -> Result<ExitCode> {
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
            .filter(|e| e.source().starts_with(&d))
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
    print_hook(&c, "pre_apply_hook", &config.hooks.pre_apply_hook);
    print_hook(&c, "post_apply_hook", &config.hooks.post_apply_hook);
    Ok(ExitCode::SUCCESS)
}

fn print_hook(c: &C, label: &str, body: &str) {
    if body.is_empty() {
        println!(
            "{}{}:{}       {}(not set){}",
            c.bold, label, c.reset, c.dim, c.reset
        );
    } else {
        let preview: String = body.lines().next().unwrap_or("").chars().take(60).collect();
        println!(
            "{}{}:{}       {}{}…{}",
            c.bold, label, c.reset, c.cyan, preview, c.reset
        );
    }
}

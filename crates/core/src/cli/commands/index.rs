use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;

use crate::config::Config;
use crate::integrations::{self, Integration, SourceKind};
use crate::paths::Paths;

use super::super::notify::{is_rofi_context, notify_send};
use super::super::style::C;

pub(in crate::cli) fn run(paths: &Paths, config: &Config, integration: &str) -> Result<ExitCode> {
    let targets: Vec<Box<dyn Integration>> = if integration == "all" {
        // `--all` rebuilds every scan-backed source. Search-backed sources
        // (booru) skip — their index is whatever the last search returned,
        // and re-reading it would be a no-op.
        integrations::all()
            .into_iter()
            .filter(|i| matches!(i.kind(), SourceKind::Scan))
            .collect()
    } else {
        vec![integrations::by_name(integration)?]
    };

    let in_rofi = is_rofi_context();
    let c = C::stderr();
    let multi = targets.len() > 1;
    let mut total = 0usize;

    let catalog = crate::tags::TagCatalog::open(paths.store())?;

    for integ in &targets {
        if in_rofi {
            notify_send(&format!("Indexing {}…", integ.name()), 3000);
        }
        let started = Instant::now();
        match integ.index(paths, config) {
            Ok(idx) => {
                let n = idx.entries.len();
                total += n;
                let elapsed = started.elapsed().as_secs_f32();
                log::info!(
                    "{}{}{} indexed {}{}{} entries in {:.2}s",
                    c.yellow, integ.name(), c.reset,
                    c.green, n, c.reset,
                    elapsed,
                );
                // Pull native tags into the catalog so the picker can suggest
                // them without re-reading the whole index. Each add() persists
                // immediately — no separate save() step.
                catalog.extend(
                    integ.name(),
                    idx.entries.iter().flat_map(|e| e.tags().iter().cloned()),
                );
                if in_rofi && multi {
                    notify_send(
                        &format!("{}: {} entries ({:.1}s)", integ.name(), n, elapsed),
                        0,
                    );
                }
            }
            Err(err) => {
                log::error!(
                    "{}{}{} index failed: {err:#}",
                    c.red, integ.name(), c.reset,
                );
                if in_rofi {
                    notify_send(&format!("{}: failed — {err}", integ.name()), 5000);
                }
            }
        }
    }

    if in_rofi {
        let msg = if multi {
            format!("Done — {total} total entries")
        } else {
            format!("Done — {total} entries indexed")
        };
        notify_send(&msg, 4000);
    }

    Ok(ExitCode::SUCCESS)
}

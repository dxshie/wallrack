//! `wallrack tag` — per-entry tag override edits, plus the catalog
//! commands (`available`, `create`, `delete`).

use std::io;
use std::process::ExitCode;

use anyhow::{Result, anyhow};

use crate::integrations;
use crate::paths::Paths;

use super::super::args::{IntegrationArg, TagCmd};
use super::super::format_list::write_string_list;
use super::super::state_helpers::resolve_integration;

pub(in crate::cli) fn run(paths: &Paths, cmd: TagCmd) -> Result<ExitCode> {
    let overrides = crate::tags::TagOverrides::open(paths.store())?;
    let catalog = crate::tags::TagCatalog::open(paths.store())?;
    match cmd {
        TagCmd::Add {
            integration,
            id,
            tag,
        } => {
            overrides.add(integration.as_str(), &id, &tag)?;
            // Newly-added tags should be immediately suggestable in the
            // picker, so reflect them in the catalog right away rather than
            // waiting for the next re-index.
            catalog.add(integration.as_str(), &tag);
        }
        TagCmd::Remove {
            integration,
            id,
            tag,
        } => {
            overrides.remove(integration.as_str(), &id, &tag)?;
        }
        TagCmd::Set {
            integration,
            id,
            tags,
        } => {
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
                    let effective = idx
                        .entries
                        .iter()
                        .find(|e| e.id() == id)
                        .map(|e| e.tags().to_vec())
                        .unwrap_or_default();
                    let prior = overrides
                        .get(integration.as_str(), &id)?
                        .unwrap_or_default();
                    // native = (effective ∪ prior.removed) \ prior.added
                    let mut native: std::collections::BTreeSet<String> =
                        effective.into_iter().collect();
                    native.extend(prior.removed.iter().cloned());
                    for added in &prior.added {
                        native.remove(added);
                    }
                    native.into_iter().collect()
                }
                Err(_) => Vec::new(),
            };
            overrides.set(integration.as_str(), &id, &tags, &native)?;
            catalog.extend(integration.as_str(), tags.iter().cloned());
        }
        TagCmd::Clear { integration, id } => {
            overrides.clear(integration.as_str(), &id)?;
        }
        TagCmd::Show { integration, id } => {
            let integ = integrations::by_name(integration.as_str())?;
            let idx = integ.read_index(paths)?;
            if let Some(entry) = idx.entries.iter().find(|e| e.id() == id) {
                for t in entry.tags() {
                    println!("{t}");
                }
            } else {
                return Err(anyhow!("entry not in index: {id}"));
            }
        }
        TagCmd::Available {
            integration,
            format,
        } => {
            let integration = resolve_integration(paths, integration.map(IntegrationArg::as_str))?;
            let tags = catalog.list(&integration);
            let stdout = io::stdout().lock();
            let mut out = std::io::BufWriter::new(stdout);
            write_string_list(&mut out, &tags, format)?;
            use std::io::Write;
            out.flush()?;
        }
        TagCmd::Create { integration, tag } => {
            catalog.add(integration.as_str(), &tag);
        }
        TagCmd::Delete {
            integration,
            cascade,
            tag,
        } => {
            catalog.remove(integration.as_str(), &tag);
            if cascade {
                // Hide the tag on every entry that currently carries it —
                // including native tags from project.json — by writing a
                // `removed` override per affected entry.
                let integ = integrations::by_name(integration.as_str())?;
                if let Ok(idx) = integ.read_index(paths) {
                    for entry in &idx.entries {
                        if entry.tags().iter().any(|t| t == &tag) {
                            overrides.remove(integration.as_str(), entry.id(), &tag)?;
                        }
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

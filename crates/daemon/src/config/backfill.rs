//! One-shot migrations for legacy on-disk agent state.
//!
//! Two migrations live here:
//!
//! 1. [`backfill_local_agent_ids`] — stamps a fresh ULID onto any
//!    `[agents.<name>]` stanza in `local/CrabTalk.toml` that predates
//!    the `id` field. Runs at daemon startup, idempotent across
//!    restarts.
//! 2. [`migrate_local_agent_prompts`] — copies `local/agents/<name>.md`
//!    into `agents/<ulid>/prompt.md` under the runtime `Storage`.
//!    Also runs at startup, also idempotent — once a prompt exists at
//!    the new key, the source file is left alone (rollback-friendly).
//!
//! Plugin manifests are intentionally left alone — they come from git
//! repos and their agents keep their fs paths.

use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, Item, value};
use ulid::Ulid;
use wcore::{ResolvedManifest, Storage, paths::LOCAL_DIR};

/// Ensure every `[agents.<name>]` entry in `local/CrabTalk.toml` has an
/// `id` field. Missing entries get a fresh ULID; the file is rewritten
/// in place (preserving comments and formatting via `toml_edit`) only
/// if at least one backfill happened.
pub fn backfill_local_agent_ids(config_dir: &Path) -> Result<()> {
    let manifest_path = config_dir.join(LOCAL_DIR).join("CrabTalk.toml");
    if !manifest_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("parse {}", manifest_path.display()))?;

    let mut mutated = 0usize;
    if let Some(agents_item) = doc.get_mut("agents")
        && let Some(agents) = agents_item.as_table_like_mut()
    {
        for (name, item) in agents.iter_mut() {
            if let Item::Table(table) = item {
                if !table.contains_key("id") {
                    let id = Ulid::new().to_string();
                    table.insert("id", value(id.clone()));
                    tracing::info!("backfilled agent id for '{name}': {id}");
                    mutated += 1;
                }
            } else if let Item::Value(toml_edit::Value::InlineTable(inline)) = item
                && !inline.contains_key("id")
            {
                let id = Ulid::new().to_string();
                inline.insert("id", toml_edit::Value::from(id.clone()));
                tracing::info!("backfilled agent id for '{name}': {id}");
                mutated += 1;
            }
        }
    }

    if mutated > 0 {
        std::fs::write(&manifest_path, doc.to_string())
            .with_context(|| format!("write {}", manifest_path.display()))?;
        tracing::info!(
            "backfilled {mutated} agent id(s) in {}",
            manifest_path.display()
        );
    }
    Ok(())
}

/// Copy `local/agents/<name>.md` prompt files into the runtime
/// [`Storage`] under `agents/<ulid>/prompt.md`. Only agents that both
/// (a) appear in the resolved manifest with a non-nil id and (b) have
/// an existing source file under `local/agents/` are migrated. The
/// source files are left in place so rollbacks stay trivial; later
/// phases (or a manual cleanup) can delete them.
pub fn migrate_local_agent_prompts(
    config_dir: &Path,
    manifest: &ResolvedManifest,
    storage: &dyn Storage,
) -> Result<()> {
    let legacy_dir = config_dir.join(wcore::paths::AGENTS_DIR);
    if !legacy_dir.exists() {
        return Ok(());
    }

    let mut migrated = 0usize;
    for (name, cfg) in &manifest.agents {
        if cfg.id.is_nil() {
            continue;
        }
        let new_key = format!("agents/{}/prompt.md", cfg.id);
        if storage.get(&new_key).ok().flatten().is_some() {
            continue;
        }
        let legacy_path = legacy_dir.join(format!("{name}.md"));
        if !legacy_path.exists() {
            continue;
        }
        let content = std::fs::read(&legacy_path)
            .with_context(|| format!("read {}", legacy_path.display()))?;
        storage
            .put(&new_key, &content)
            .with_context(|| format!("write storage key {new_key}"))?;
        tracing::info!("migrated agent prompt '{name}' -> {new_key}");
        migrated += 1;
    }
    if migrated > 0 {
        tracing::info!("migrated {migrated} local agent prompt(s) into Storage");
    }
    Ok(())
}

/// Storage key for an agent's prompt given its ULID-formatted id.
pub fn agent_prompt_key(id: &str) -> String {
    format!("agents/{id}/prompt.md")
}

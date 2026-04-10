//! One-shot migrations for legacy on-disk agent state.

use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, Item, value};
use ulid::Ulid;
use wcore::{ResolvedManifest, paths::LOCAL_DIR, repos::Storage};

/// Ensure every `[agents.<name>]` entry in `local/CrabTalk.toml` has an
/// `id` field. Missing entries get a fresh ULID.
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

/// Copy `local/agents/<name>.md` prompt files into the agent repo under
/// ULID-keyed paths. Idempotent — skips agents whose repo entry already
/// has a prompt.
pub fn migrate_local_agent_prompts(
    config_dir: &Path,
    manifest: &ResolvedManifest,
    storage: &impl Storage,
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
        // Check if prompt already exists in repo.
        if let Ok(Some(loaded)) = storage.load_agent(&cfg.id)
            && !loaded.system_prompt.is_empty()
        {
            continue;
        }
        let legacy_path = legacy_dir.join(format!("{name}.md"));
        if !legacy_path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&legacy_path)
            .with_context(|| format!("read {}", legacy_path.display()))?;
        storage
            .upsert_agent(cfg, &content)
            .with_context(|| format!("migrate prompt for agent '{name}'"))?;
        tracing::info!(
            "migrated agent prompt '{name}' -> agents/{}/prompt.md",
            cfg.id
        );
        migrated += 1;
    }
    if migrated > 0 {
        tracing::info!("migrated {migrated} local agent prompt(s)");
    }
    Ok(())
}

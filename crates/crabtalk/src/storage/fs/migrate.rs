//! One-shot migrations applied to `local/settings.toml` before the daemon
//! deserializes it.
//!
//! Each migration detects its own legacy shape and is a no-op once the file
//! has been upgraded, so calling `migrate_settings` on every startup is safe
//! and cheap.

use super::{FsStorage, SETTINGS_HEADER, atomic_write};
use anyhow::Result;
use std::io::ErrorKind;
use tokio::fs;
use toml::{Table, Value};

/// Run all settings-file migrations.
pub(crate) async fn migrate_settings(storage: &FsStorage) -> Result<()> {
    let path = storage.settings_path();
    let raw = match fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut value: Value = toml::from_str(&raw)?;
    let Some(table) = value.as_table_mut() else {
        return Ok(());
    };

    let mut changed = false;
    if inline_agent_mcps(table) {
        changed = true;
    }

    if changed {
        let body = toml::to_string_pretty(&value)?;
        let mut content = String::with_capacity(SETTINGS_HEADER.len() + body.len());
        content.push_str(SETTINGS_HEADER);
        content.push_str(&body);
        atomic_write(&path, content.as_bytes()).await?;
        tracing::info!("migrated settings.toml: inlined per-agent MCP configs");
    }
    Ok(())
}

/// Replace the legacy `agents.<n>.mcps = ["name", …]` form with inline
/// `McpServerConfig` tables, drawing from the top-level `[mcps.<name>]`
/// registry. After migration the global `[mcps]` section is dropped.
///
/// Returns `true` when the file was modified.
fn inline_agent_mcps(table: &mut Table) -> bool {
    // Snapshot the global registry first; we'll consume it.
    let registry: Table = match table.remove("mcps") {
        Some(Value::Table(t)) => t,
        Some(other) => {
            // Unrecognized shape — preserve.
            table.insert("mcps".to_string(), other);
            return false;
        }
        None => Table::new(),
    };

    let Some(agents_value) = table.get_mut("agents") else {
        // Nothing references the registry; if it existed at all, dropping it
        // is the migration. Empty registry → unchanged.
        return !registry.is_empty();
    };
    let Some(agents) = agents_value.as_table_mut() else {
        return !registry.is_empty();
    };

    let mut changed = !registry.is_empty();
    for (_agent, agent_value) in agents.iter_mut() {
        let Some(agent_table) = agent_value.as_table_mut() else {
            continue;
        };
        let Some(mcps_value) = agent_table.get_mut("mcps") else {
            continue;
        };
        let Some(items) = mcps_value.as_array_mut() else {
            continue;
        };
        if !items.iter().any(|v| v.is_str()) {
            continue;
        }
        let mut migrated: Vec<Value> = Vec::with_capacity(items.len());
        for item in items.drain(..) {
            match item {
                Value::String(name) => match registry.get(&name) {
                    Some(Value::Table(cfg)) => {
                        let mut cfg = cfg.clone();
                        // The on-disk shape keys by name; backfill the field
                        // so it round-trips into McpServerConfig cleanly.
                        cfg.entry("name".to_string())
                            .or_insert_with(|| Value::String(name.clone()));
                        migrated.push(Value::Table(cfg));
                    }
                    _ => {
                        tracing::warn!(
                            "agent referenced unknown MCP '{name}'; dropping during migration"
                        );
                    }
                },
                other => migrated.push(other),
            }
        }
        *items = migrated;
        changed = true;
    }
    changed
}

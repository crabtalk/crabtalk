//! One-shot migrations applied to on-disk state before the daemon reads it —
//! `local/settings.toml`, and the session directory layout.
//!
//! Each migration detects its own legacy shape and is a no-op once upgraded, so
//! calling them on every startup is safe and cheap.

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

/// Rename legacy `<agent>_<sender>_<seq>` session directories to opaque ULIDs.
///
/// The old name encoded the conversation's identity, which is why renaming an
/// agent orphaned its transcripts. Everything the name carried is already in
/// each session's `meta`, so this drops nothing: the association simply moves
/// from the path to the data.
///
/// A directory that does not parse as the legacy triple is left alone, which is
/// what makes this idempotent — a ULID has no `_`, so a second run sees nothing
/// to do. A session with no readable `meta` is also left alone: its identity
/// lives only in its name, and `list_sessions` already ignores it.
pub(crate) async fn migrate_sessions(storage: &FsStorage) -> Result<()> {
    let root = &storage.sessions_root;
    if !root.exists() {
        return Ok(());
    }

    let mut legacy = Vec::new();
    let mut entries = fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_legacy_slug(&name) || !entry.file_type().await?.is_dir() {
            continue;
        }
        // No meta, no identity to carry over — moving it would strand it under
        // a name nothing can resolve.
        if fs::read(entry.path().join("meta")).await.is_err() {
            tracing::warn!("leaving session '{name}' in place: no readable meta");
            continue;
        }
        legacy.push(name);
    }

    if legacy.is_empty() {
        return Ok(());
    }
    for name in &legacy {
        let to = root.join(ulid::Ulid::new().to_string());
        fs::rename(root.join(name), &to).await?;
    }
    tracing::info!(
        "migrated {} session directories to opaque ids",
        legacy.len()
    );
    Ok(())
}

/// `<agent>_<sender>_<seq>`, the pre-ULID session directory name.
fn is_legacy_slug(name: &str) -> bool {
    let Some((_, seq)) = name.rsplit_once('_') else {
        return false;
    };
    !seq.is_empty() && seq.chars().all(|c| c.is_ascii_digit())
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

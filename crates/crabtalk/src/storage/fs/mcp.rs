//! MCP server registry — persisted in `local/settings.toml` under
//! `[mcps.<name>]`.

use super::FsStorage;
use anyhow::Result;
use std::collections::BTreeMap;
use wcore::{McpServerConfig, storage::validate_table_name};

pub(super) async fn list_mcps(storage: &FsStorage) -> Result<BTreeMap<String, McpServerConfig>> {
    let mut file = storage.read_settings().await?;
    // Fill in `name` from the key for entries hand-edited into the
    // file without one. In-memory only — caller sees normalized
    // values but the file on disk is left as-is.
    for (name, cfg) in file.mcps.iter_mut() {
        if cfg.name.is_empty() {
            cfg.name = name.clone();
        }
    }
    Ok(file.mcps)
}

pub(super) async fn load_mcp(storage: &FsStorage, name: &str) -> Result<Option<McpServerConfig>> {
    Ok(storage
        .read_settings()
        .await?
        .mcps
        .remove(name)
        .map(|mut cfg| {
            if cfg.name.is_empty() {
                cfg.name = name.to_owned();
            }
            cfg
        }))
}

pub(super) async fn upsert_mcp(storage: &FsStorage, config: &McpServerConfig) -> Result<()> {
    validate_table_name("mcp", &config.name)?;
    let mut file = storage.read_settings().await?;
    file.mcps.insert(config.name.clone(), config.clone());
    storage.write_settings(&file).await
}

pub(super) async fn delete_mcp(storage: &FsStorage, name: &str) -> Result<bool> {
    let mut file = storage.read_settings().await?;
    let removed = file.mcps.remove(name).is_some();
    if removed {
        storage.write_settings(&file).await?;
    }
    Ok(removed)
}

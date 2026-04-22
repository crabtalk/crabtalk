//! Agent persistence — definitions in `local/settings.toml` under
//! `[agents.<name>]`, system prompt in `agents/<ulid>/prompt.md`.

use super::{FsStorage, atomic_write};
use anyhow::Result;
use std::{fs, io::ErrorKind, path::PathBuf};
use wcore::{AgentConfig, AgentId, storage::validate_table_name};

fn agent_prompt_path(storage: &FsStorage, id: &AgentId) -> PathBuf {
    storage
        .config_dir
        .join("agents")
        .join(id.to_string())
        .join("prompt.md")
}

fn read_agent_prompt(storage: &FsStorage, id: &AgentId) -> Option<String> {
    if id.is_nil() {
        return None;
    }
    fs::read_to_string(agent_prompt_path(storage, id)).ok()
}

pub(super) fn list_agents(storage: &FsStorage) -> Result<Vec<AgentConfig>> {
    let file = storage.read_settings()?;
    let mut out = Vec::with_capacity(file.agents.len());
    for (name, mut cfg) in file.agents {
        cfg.name = name;
        cfg.system_prompt = read_agent_prompt(storage, &cfg.id).unwrap_or_default();
        out.push(cfg);
    }
    Ok(out)
}

pub(super) fn load_agent(storage: &FsStorage, id: &AgentId) -> Result<Option<AgentConfig>> {
    if id.is_nil() {
        return Ok(None);
    }
    let file = storage.read_settings()?;
    let Some((name, mut cfg)) = file.agents.into_iter().find(|(_, c)| c.id == *id) else {
        return Ok(None);
    };
    cfg.name = name;
    cfg.system_prompt = read_agent_prompt(storage, id).unwrap_or_default();
    Ok(Some(cfg))
}

pub(super) fn load_agent_by_name(storage: &FsStorage, name: &str) -> Result<Option<AgentConfig>> {
    let file = storage.read_settings()?;
    let Some(mut cfg) = file.agents.get(name).cloned() else {
        return Ok(None);
    };
    cfg.name = name.to_owned();
    cfg.system_prompt = read_agent_prompt(storage, &cfg.id).unwrap_or_default();
    Ok(Some(cfg))
}

pub(super) fn upsert_agent(storage: &FsStorage, config: &AgentConfig, prompt: &str) -> Result<()> {
    if config.id.is_nil() {
        anyhow::bail!("cannot upsert agent with nil ID");
    }
    if config.name.is_empty() {
        anyhow::bail!("cannot upsert agent with empty name");
    }
    validate_table_name("agent", &config.name)?;
    let mut file = storage.read_settings()?;
    file.agents.insert(config.name.clone(), config.clone());
    storage.write_settings(&file)?;
    let path = agent_prompt_path(storage, &config.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(&path, prompt.as_bytes())
}

pub(super) fn delete_agent(storage: &FsStorage, id: &AgentId) -> Result<bool> {
    let mut file = storage.read_settings()?;
    let removed_name = file
        .agents
        .iter()
        .find(|(_, c)| c.id == *id)
        .map(|(n, _)| n.clone());
    let settings_removed = removed_name.is_some();
    if let Some(name) = removed_name {
        file.agents.remove(&name);
        storage.write_settings(&file)?;
    }
    let dir = storage.config_dir.join("agents").join(id.to_string());
    let dir_removed = match fs::remove_dir_all(&dir) {
        Ok(()) => true,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    Ok(dir_removed || settings_removed)
}

pub(super) fn rename_agent(storage: &FsStorage, id: &AgentId, new_name: &str) -> Result<bool> {
    validate_table_name("agent", new_name)?;
    let mut file = storage.read_settings()?;
    let old_name = file
        .agents
        .iter()
        .find(|(_, c)| c.id == *id)
        .map(|(n, _)| n.clone());
    let Some(old_name) = old_name else {
        return Ok(false);
    };
    if old_name == new_name {
        return Ok(true);
    }
    if file.agents.contains_key(new_name) {
        anyhow::bail!("agent '{new_name}' already exists");
    }
    let cfg = file.agents.remove(&old_name).expect("present above");
    file.agents.insert(new_name.to_owned(), cfg);
    storage.write_settings(&file)?;
    Ok(true)
}

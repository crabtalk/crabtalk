//! Agent persistence — definitions in `local/settings.toml` under
//! `[agents.<name>]`, whole. The description is the system message and
//! serializes with everything else, so there is no second file to keep in
//! step with the first.

use crate::fs::FsStorage;
use anyhow::Result;
use std::io::ErrorKind;
use tokio::fs;
use wcore::{AgentConfig, AgentId, storage::validate_table_name};

impl FsStorage {
    pub(super) async fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        let file = self.read_settings().await?;
        let mut out = Vec::with_capacity(file.agents.len());
        for (name, mut cfg) in file.agents {
            cfg.name = name;
            out.push(cfg);
        }
        Ok(out)
    }

    pub(super) async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        if id.is_nil() {
            return Ok(None);
        }
        let file = self.read_settings().await?;
        let Some((name, mut cfg)) = file.agents.into_iter().find(|(_, c)| c.id == *id) else {
            return Ok(None);
        };
        cfg.name = name;
        Ok(Some(cfg))
    }

    pub(super) async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let file = self.read_settings().await?;
        let Some(mut cfg) = file.agents.get(name).cloned() else {
            return Ok(None);
        };
        cfg.name = name.to_owned();
        Ok(Some(cfg))
    }

    pub(super) async fn upsert_agent(&self, config: &AgentConfig) -> Result<()> {
        if config.id.is_nil() {
            anyhow::bail!("cannot upsert agent with nil ID");
        }
        if config.name.is_empty() {
            anyhow::bail!("cannot upsert agent with empty name");
        }
        validate_table_name("agent", &config.name)?;
        let mut file = self.read_settings().await?;
        file.agents.insert(config.name.clone(), config.clone());
        self.write_settings(&file).await
    }

    pub(super) async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let mut file = self.read_settings().await?;
        let removed_name = file
            .agents
            .iter()
            .find(|(_, c)| c.id == *id)
            .map(|(n, _)| n.clone());
        let settings_removed = removed_name.is_some();
        if let Some(name) = removed_name {
            file.agents.remove(&name);
            self.write_settings(&file).await?;
        }
        let dir = self.config_dir.join("agents").join(id.to_string());
        let dir_removed = match fs::remove_dir_all(&dir).await {
            Ok(()) => true,
            Err(e) if e.kind() == ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        Ok(dir_removed || settings_removed)
    }

    pub(super) async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        validate_table_name("agent", new_name)?;
        let mut file = self.read_settings().await?;
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
        self.write_settings(&file).await?;
        Ok(true)
    }
}

//! Filesystem-backed [`AgentRepo`] implementation.
//!
//! Layout:
//! - Agent configs live in the local manifest TOML (`[agents.<name>]` stanzas).
//! - Prompts live at `agents/<ulid>/prompt.md` under the config dir.
//! - Legacy prompts from `<agent_dir>/<name>.md` are also consulted.

use anyhow::Result;
use std::{fs, io::ErrorKind, path::PathBuf};
use wcore::{AgentConfig, AgentId, repos::AgentRepo};

pub struct FsAgentRepo {
    /// Config directory root (for prompt storage under `agents/<ulid>/`).
    config_dir: PathBuf,
    /// Directories containing `<name>.md` legacy prompt files.
    agent_dirs: Vec<PathBuf>,
}

impl FsAgentRepo {
    pub fn new(config_dir: PathBuf, agent_dirs: Vec<PathBuf>) -> Self {
        Self {
            config_dir,
            agent_dirs,
        }
    }

    fn prompt_path(&self, id: &AgentId) -> PathBuf {
        self.config_dir
            .join("agents")
            .join(id.to_string())
            .join("prompt.md")
    }
}

impl AgentRepo for FsAgentRepo {
    fn list(&self) -> Result<Vec<AgentConfig>> {
        // The agent list comes from the manifest (TOML config), not from
        // scanning directories. The daemon's load_agents path handles this
        // at a higher level. This method returns an empty list — the
        // daemon orchestrates agent loading from manifests + prompts.
        Ok(Vec::new())
    }

    fn load(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        if id.is_nil() {
            return Ok(None);
        }
        let path = self.prompt_path(id);
        match fs::read_to_string(&path) {
            Ok(prompt) => Ok(Some(AgentConfig {
                id: *id,
                system_prompt: prompt,
                ..Default::default()
            })),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn load_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        // Name-based lookup requires the manifest (TOML). The daemon
        // handles this at a higher level via resolve_agent_prompt.
        for dir in &self.agent_dirs {
            let path = dir.join(format!("{name}.md"));
            if let Ok(prompt) = fs::read_to_string(&path) {
                let mut config = AgentConfig::new(name);
                config.system_prompt = prompt;
                return Ok(Some(config));
            }
        }
        Ok(None)
    }

    fn upsert(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        if config.id.is_nil() {
            anyhow::bail!("cannot upsert agent with nil ID");
        }
        let path = self.prompt_path(&config.id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::repos::atomic_write(&path, prompt.as_bytes())?;
        Ok(())
    }

    fn delete(&self, id: &AgentId) -> Result<bool> {
        let dir = self.config_dir.join("agents").join(id.to_string());
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn rename(&self, _id: &AgentId, _new_name: &str) -> Result<bool> {
        // Rename is a manifest-level operation (change the TOML key).
        // The ULID stays stable, so the prompt file doesn't move.
        // The daemon protocol handler does the TOML edit.
        Ok(true)
    }
}

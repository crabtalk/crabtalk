//! Agent registry — persistent and ephemeral agent management.

use super::Runtime;
use crate::{Config, Env, Hook};
use anyhow::Result;
use std::sync::Arc;
use wcore::{Agent, AgentBuilder, AgentConfig, AgentId, ToolDispatcher, paths, storage::Storage};

impl<C: Config> Runtime<C> {
    pub fn add_agent(&self, config: AgentConfig) {
        let _ = self.upsert_agent(config);
    }

    pub fn upsert_agent(&self, config: AgentConfig) -> AgentConfig {
        let (name, agent) = self.build_agent(config);
        let registered = agent.config.clone();
        // Fire the hook before insert so the invariant "visible via .agent()
        // ⇒ tracked by hooks" holds. Same rationale in reverse for remove_agent.
        self.env.hook().on_register_agent(&name, &registered);
        self.agents.write().insert(name, agent);
        registered
    }

    pub fn remove_agent(&self, name: &str) -> bool {
        let removed = self.agents.write().remove(name).is_some();
        if removed {
            self.env.hook().on_unregister_agent(name);
        }
        removed
    }

    fn build_agent(&self, config: AgentConfig) -> (String, Agent<C::Provider>) {
        let config = self.env.hook().on_build_agent(config);
        let name = config.name.clone();
        let tools = self.tools.filtered_snapshot(&config.tools);
        let dispatcher: Arc<dyn ToolDispatcher> = self.env.clone();
        let agent = AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools)
            .dispatcher(dispatcher)
            .build();
        (name, agent)
    }

    pub fn agent(&self, name: &str) -> Option<AgentConfig> {
        self.agents.read().get(name).map(|a| a.config.clone())
    }

    pub fn agents(&self) -> Vec<AgentConfig> {
        self.agents
            .read()
            .values()
            .map(|a| a.config.clone())
            .collect()
    }

    // --- Ephemeral agents ---

    pub async fn add_ephemeral(&self, config: AgentConfig) {
        let (name, agent) = self.build_agent(config);
        self.ephemeral_agents.write().await.insert(name, agent);
    }

    pub async fn remove_ephemeral(&self, name: &str) {
        self.ephemeral_agents.write().await.remove(name);
    }

    pub(crate) async fn resolve_agent(&self, name: &str) -> Option<Agent<C::Provider>> {
        let persistent = self.agents.read().get(name).cloned();
        if persistent.is_some() {
            return persistent;
        }
        self.ephemeral_agents.read().await.get(name).cloned()
    }

    pub(crate) async fn has_agent(&self, name: &str) -> bool {
        let has_persistent = self.agents.read().contains_key(name);
        if has_persistent {
            return true;
        }
        self.ephemeral_agents.read().await.contains_key(name)
    }

    // --- Storage-backed CRUD ---

    /// Create a new persisted agent. Writes storage, registers in the
    /// runtime, returns the registered config.
    pub async fn create_agent(&self, mut config: AgentConfig, prompt: &str) -> Result<AgentConfig> {
        validate_agent_name(&config.name)?;
        if config.id.is_nil() {
            config.id = AgentId::new();
        }
        let storage = self.storage();
        if storage.load_agent_by_name(&config.name).await?.is_some() {
            anyhow::bail!("agent '{}' already exists", config.name);
        }
        storage.upsert_agent(&config, prompt).await?;
        self.load_and_register(&config.name).await
    }

    /// Update an existing persisted agent (or create if absent). Writes
    /// storage, re-registers in the runtime, returns the registered config.
    pub async fn update_agent(&self, mut config: AgentConfig, prompt: &str) -> Result<AgentConfig> {
        validate_agent_name(&config.name)?;
        let storage = self.storage();
        let existing = storage.load_agent_by_name(&config.name).await?;
        if let Some(prev) = &existing {
            if config.id.is_nil() {
                config.id = prev.id;
            }
        } else if config.id.is_nil() {
            config.id = AgentId::new();
        }
        let prompt = if prompt.is_empty() {
            existing.map(|a| a.system_prompt).unwrap_or_default()
        } else {
            prompt.to_owned()
        };
        storage.upsert_agent(&config, &prompt).await?;
        self.load_and_register(&config.name).await
    }

    /// Purge a persisted agent — removes from storage AND unregisters from
    /// the runtime. Named distinctly from `Storage::delete_agent` (which is
    /// storage-only and keyed by `AgentId`) to avoid confusion about which
    /// layer cascades.
    pub async fn purge_agent(&self, name: &str) -> Result<bool> {
        let storage = self.storage();
        let Some(existing) = storage.load_agent_by_name(name).await? else {
            return Ok(false);
        };
        let removed = storage.delete_agent(&existing.id).await?;
        if removed {
            self.remove_agent(name);
        }
        Ok(removed)
    }

    /// Rename a persisted agent. Updates storage, re-registers under the
    /// new name in the runtime.
    pub async fn rename_agent(&self, old_name: &str, new_name: &str) -> Result<AgentConfig> {
        validate_agent_name(new_name)?;
        anyhow::ensure!(
            old_name != paths::DEFAULT_AGENT,
            "cannot rename the default agent '{old_name}'"
        );
        // Short-circuit rename-to-same: returns the in-memory config without
        // round-tripping through storage. Callers that expected a storage
        // refresh should do an explicit read.
        if old_name == new_name {
            return self
                .agent(old_name)
                .ok_or_else(|| anyhow::anyhow!("agent '{old_name}' not found"));
        }
        let storage = self.storage();
        let existing = storage
            .load_agent_by_name(old_name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{old_name}' not found"))?;
        storage.rename_agent(&existing.id, new_name).await?;
        self.remove_agent(old_name);
        self.load_and_register(new_name).await
    }

    async fn load_and_register(&self, name: &str) -> Result<AgentConfig> {
        let config = self
            .storage()
            .load_agent_by_name(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{name}' missing from storage after upsert"))?;
        Ok(self.upsert_agent(config))
    }
}

fn validate_agent_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "agent name cannot be empty");
    anyhow::ensure!(
        !name.contains('/') && !name.contains('\\') && !name.contains(".."),
        "agent name '{name}' contains invalid characters"
    );
    Ok(())
}

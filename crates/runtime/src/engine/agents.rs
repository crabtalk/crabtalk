//! Agent registry — persistent and ephemeral agent management.

use super::Runtime;
use crate::{Agent, AgentBuilder, ToolDispatcher};
use crate::{Config, Env, Harness};
use anyhow::Result;
use std::sync::Arc;
use storage::{AgentConfig, AgentId, Storage};

impl<C: Config> Runtime<C> {
    pub fn add_agent(&self, config: AgentConfig) {
        let _ = self.upsert_agent(config);
    }

    pub fn upsert_agent(&self, config: AgentConfig) -> AgentConfig {
        let (id, agent) = self.build_agent(config);
        let registered = agent.config.clone();
        // Fire the hook before insert so the invariant "visible via .agent()
        // ⇒ tracked by hooks" holds. Same rationale in reverse for remove_agent.
        self.env.hook().on_register_agent(&id, &registered);
        self.agents.write().insert(id, agent);
        registered
    }

    pub fn remove_agent(&self, id: &AgentId) -> bool {
        let removed = self.agents.write().remove(id).is_some();
        if removed {
            self.env.hook().on_unregister_agent(id);
        }
        removed
    }

    fn build_agent(&self, config: AgentConfig) -> (AgentId, Agent<C::Provider>) {
        let config = self.env.hook().on_build_agent(config);
        let id = config.id;
        let tools = self.tools.filtered_snapshot(&config.tools);
        let dispatcher: Arc<dyn ToolDispatcher> = self.env.clone();
        let agent = AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools)
            .dispatcher(dispatcher)
            .build();
        (id, agent)
    }

    pub fn agent(&self, id: &AgentId) -> Option<AgentConfig> {
        self.agents.read().get(id).map(|a| a.config.clone())
    }

    /// Resolve a name to the agent wearing it — the runtime's only
    /// name lookup. A surface calls it once, then addresses everything
    /// else by id.
    pub fn agent_by_name(&self, name: &str) -> Option<AgentConfig> {
        self.agents
            .read()
            .values()
            .find(|a| a.config.name == name)
            .map(|a| a.config.clone())
    }

    pub fn agents(&self) -> Vec<AgentConfig> {
        self.agents
            .read()
            .values()
            .map(|a| a.config.clone())
            .collect()
    }

    pub(crate) fn resolve_agent(&self, id: &AgentId) -> Option<Agent<C::Provider>> {
        self.agents.read().get(id).cloned()
    }

    pub(crate) fn has_agent(&self, id: &AgentId) -> bool {
        self.agents.read().contains_key(id)
    }

    // --- Storage-backed CRUD ---

    /// Create a new persisted agent. Writes storage, registers in the
    /// runtime, returns the registered config.
    pub async fn create_agent(&self, mut config: AgentConfig) -> Result<AgentConfig> {
        // Identity is the daemon's to mint. An id arriving in the body
        // would make `create` a way to address an agent that exists.
        config.id = AgentId::new();
        let storage = self.storage();
        if storage.load_agent_by_name(&config.name).await?.is_some() {
            anyhow::bail!("agent '{}' already exists", config.name);
        }
        storage.upsert_agent(&config).await?;
        self.load_and_register(&config.id).await
    }

    /// Update an existing persisted agent. `id` is the identity — the
    /// one on `config` is overwritten with it, so a stale or absent id
    /// in a deserialized body cannot retarget the write.
    pub async fn update_agent(&self, id: &AgentId, mut config: AgentConfig) -> Result<AgentConfig> {
        config.id = *id;
        self.storage().upsert_agent(&config).await?;
        self.load_and_register(id).await
    }

    /// Purge a persisted agent — removes from storage AND unregisters from
    /// the runtime. Named distinctly from `Storage::delete_agent` (which is
    /// storage-only) to say which layer cascades.
    pub async fn purge_agent(&self, id: &AgentId) -> Result<bool> {
        let storage = self.storage();
        let removed = storage.delete_agent(id).await?;
        if removed {
            self.remove_agent(id);
            // A session belongs to the agent by id, so nothing can reach
            // these once it is gone — including an agent later created
            // under the same name, which gets its own id.
            let purged = storage.delete_sessions_of(id).await?;
            if purged > 0 {
                tracing::info!("purged {purged} session(s) of deleted agent '{id}'");
            }
        }
        Ok(removed)
    }

    /// Rename a persisted agent. Nothing but the label moves: sessions
    /// are keyed by the id, which the rename leaves alone.
    pub async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<AgentConfig> {
        if !self.storage().rename_agent(id, new_name).await? {
            anyhow::bail!("agent '{id}' not found");
        }
        self.load_and_register(id).await
    }

    async fn load_and_register(&self, id: &AgentId) -> Result<AgentConfig> {
        let config = self
            .storage()
            .load_agent(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent '{id}' missing from storage after write"))?;
        Ok(self.upsert_agent(config))
    }
}

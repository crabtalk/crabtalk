//! Agent resolution and persisted agent CRUD.
//!
//! There is no registry. `resolve_agent` reads the config the run in
//! front of it needs, builds an `Agent`, and hands it over; nothing is
//! kept afterwards. "Registered" is no longer a state an agent can be
//! in — it is in storage or it is not — so the hook that tracks
//! per-agent state fires on resolution instead, which is why
//! [`Harness::on_resolve_agent`](crate::Harness::on_resolve_agent) must
//! be idempotent.

use crate::{Agent, AgentBuilder, Config, Env, Harness, ToolDispatcher, engine::Runtime};
use anyhow::Result;
use std::sync::Arc;
use store::{
    AgentConfig, AgentId,
    interface::{Agents, Sessions},
};

impl<C: Config> Runtime<C> {
    /// One agent's config, or `None` if storage has no such agent.
    pub async fn agent(&self, id: &AgentId) -> Option<AgentConfig> {
        self.storage().load_agent(id).await.ok().flatten()
    }

    /// Every agent's config.
    ///
    /// Two round trips by design: the index hands back ids, and each
    /// config is its own read. A single query returning them all would
    /// mean every system prompt in the store crossing the wire to render
    /// a list of names.
    pub async fn agents(&self) -> Vec<AgentConfig> {
        let Ok(ids) = self.storage().agent_ids().await else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(config) = self.agent(&id).await {
                out.push(config);
            }
        }
        out
    }

    pub(crate) async fn has_agent(&self, id: &AgentId) -> bool {
        self.agent(id).await.is_some()
    }

    /// Every agent's id and name, for listings that put a name beside an
    /// id. Reads each config once, however many rows are being labelled.
    pub async fn agent_names(&self) -> std::collections::HashMap<AgentId, String> {
        self.agents()
            .await
            .into_iter()
            .map(|config| (config.id, config.name))
            .collect()
    }

    /// Build the agent for a run.
    ///
    /// The hook fires before the `Agent` exists, so anything tracking
    /// per-agent state has it in place by the time the run starts —
    /// the same ordering the registry used to guarantee, now scoped to
    /// the agent actually running rather than every agent that exists.
    pub(crate) async fn resolve_agent(&self, id: &AgentId) -> Option<Agent<C::Provider>> {
        let config = self.agent(id).await?;
        self.env.hook().on_resolve_agent(id, &config);
        Some(self.build_agent(config))
    }

    fn build_agent(&self, config: AgentConfig) -> Agent<C::Provider> {
        let config = self.env.hook().on_build_agent(config);
        let tools = self.tools.filtered_snapshot(&config.tools);
        let dispatcher: Arc<dyn ToolDispatcher> = self.env.clone();
        AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools)
            .dispatcher(dispatcher)
            .build()
    }

    // --- Storage-backed CRUD ---

    /// Create a new persisted agent.
    pub async fn create_agent(&self, mut config: AgentConfig) -> Result<AgentConfig> {
        // Identity is the daemon's to mint. An id arriving in the body
        // would make `create` a way to address an agent that exists.
        config.id = AgentId::new();
        let storage = self.storage();
        if storage.load_agent_by_name(&config.name).await?.is_some() {
            anyhow::bail!("agent '{}' already exists", config.name);
        }
        storage.upsert_agent(&config).await?;
        self.reload_agent(&config.id).await
    }

    /// Update an existing persisted agent. `id` is the identity — the
    /// one on `config` is overwritten with it, so a stale or absent id
    /// in a deserialized body cannot retarget the write.
    pub async fn update_agent(&self, id: &AgentId, mut config: AgentConfig) -> Result<AgentConfig> {
        config.id = *id;
        self.storage().upsert_agent(&config).await?;
        self.reload_agent(id).await
    }

    /// Purge a persisted agent and everything keyed to it.
    pub async fn purge_agent(&self, id: &AgentId) -> Result<bool> {
        let storage = self.storage();
        let removed = storage.delete_agent(id).await?;
        if removed {
            self.env.hook().on_forget_agent(id);
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
        self.reload_agent(id).await
    }

    /// Read back what was just written, so a caller sees the stored
    /// record rather than the one it sent.
    async fn reload_agent(&self, id: &AgentId) -> Result<AgentConfig> {
        self.agent(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("agent '{id}' missing from storage after write"))
    }
}

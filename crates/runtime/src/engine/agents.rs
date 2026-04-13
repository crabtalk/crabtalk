//! Agent registry — persistent and ephemeral agent management.

use crate::{Config, Env, Hook};
use std::sync::Arc;
use wcore::{Agent, AgentBuilder, AgentConfig, ToolDispatcher};

use super::Runtime;

impl<C: Config> Runtime<C> {
    pub fn add_agent(&self, config: AgentConfig) {
        let _ = self.upsert_agent(config);
    }

    pub fn upsert_agent(&self, config: AgentConfig) -> AgentConfig {
        let (name, agent) = self.build_agent(config);
        let registered = agent.config.clone();
        self.agents
            .write()
            .expect("agents lock poisoned")
            .insert(name, agent);
        registered
    }

    pub fn remove_agent(&self, name: &str) -> bool {
        self.agents
            .write()
            .expect("agents lock poisoned")
            .remove(name)
            .is_some()
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
        self.agents
            .read()
            .expect("agents lock poisoned")
            .get(name)
            .map(|a| a.config.clone())
    }

    pub fn agents(&self) -> Vec<AgentConfig> {
        self.agents
            .read()
            .expect("agents lock poisoned")
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
        let persistent = self
            .agents
            .read()
            .expect("agents lock poisoned")
            .get(name)
            .cloned();
        if persistent.is_some() {
            return persistent;
        }
        self.ephemeral_agents.read().await.get(name).cloned()
    }

    pub(crate) async fn has_agent(&self, name: &str) -> bool {
        let has_persistent = self
            .agents
            .read()
            .expect("agents lock poisoned")
            .contains_key(name);
        if has_persistent {
            return true;
        }
        self.ephemeral_agents.read().await.contains_key(name)
    }
}

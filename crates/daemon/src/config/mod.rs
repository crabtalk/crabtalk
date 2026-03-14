//! Daemon configuration loaded from TOML.

pub use crate::hook::{
    mcp::McpServerConfig,
    os::{PermissionConfig, ToolPermission},
    task::TasksConfig,
};
pub use ::model::{ModelConfig, ProviderConfig, ProviderManager};
use anyhow::Result;
pub use gateway::GatewayConfig;
pub use loader::{load_agents_dir, scaffold_config_dir};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use wcore::{
    AgentConfig, HeartbeatConfig,
    paths::{AGENTS_DIR, CONFIG_DIR, DATA_DIR, MEMORY_DB, SKILLS_DIR, SOCKET_PATH},
};

mod loader;

/// Top-level daemon configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonConfig {
    /// The walrus daemon's own agent config (model, heartbeat).
    #[serde(default)]
    pub walrus: AgentConfig,
    /// Model configurations (remote API endpoints + local models).
    #[serde(default)]
    pub model: ModelConfig,
    /// Gateway configurations (Telegram, Discord bots).
    #[serde(default)]
    pub gateway: GatewayConfig,
    /// MCP server configurations.
    #[serde(default)]
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// Task executor pool configuration.
    #[serde(default)]
    pub tasks: TasksConfig,
    /// Per-agent configurations (name → config).
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
    /// Permission configuration: global defaults + per-agent overrides.
    #[serde(default)]
    pub permissions: PermissionConfig,
    /// Managed child services (`[services.<name>]`).
    #[serde(default)]
    pub services: BTreeMap<String, crate::service::ServiceConfig>,
}

impl DaemonConfig {
    /// Parse a TOML string into a `DaemonConfig`.
    pub fn from_toml(toml_str: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(toml_str)?;
        config.model.remotes.iter_mut().for_each(|(key, provider)| {
            if provider.name.is_empty() {
                provider.name = key.clone();
            }
        });
        config.mcps.iter_mut().for_each(|(name, server)| {
            if server.name.is_empty() {
                server.name = name.clone().into();
            }
        });
        if config.walrus.model.is_none() {
            config.walrus.model = Some(::model::default_model().into());
        }
        Ok(config)
    }

    /// Load configuration from a file path.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }
}

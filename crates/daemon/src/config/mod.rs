//! Daemon configuration loaded from TOML.

pub use crate::hook::{mcp::McpServerConfig, system::SystemConfig};
pub use ::model::{ProviderDef, ProviderRegistry, validate_providers};
use anyhow::Result;
pub use loader::{DEFAULT_CONFIG, load_agents_dir, scaffold_config_dir};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use wcore::{
    AgentConfig, HeartbeatConfig,
    paths::{AGENTS_DIR, CONFIG_DIR, CONFIG_FILE, SKILLS_DIR, SOCKET_PATH},
};

mod loader;

/// Top-level daemon configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonConfig {
    /// Provider definitions (`[provider.<name>]`).
    #[serde(default)]
    pub provider: BTreeMap<String, ProviderDef>,
    /// MCP server configurations.
    #[serde(default)]
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// System configuration (tasks + memory).
    #[serde(default)]
    pub system: SystemConfig,
    /// Per-agent configurations (name → config).
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,
}

impl DaemonConfig {
    /// Parse a TOML string into a `DaemonConfig`.
    pub fn from_toml(toml_str: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(toml_str)?;
        config.mcps.iter_mut().for_each(|(name, server)| {
            if server.name.is_empty() {
                server.name = name.clone();
            }
        });
        validate_providers(&config.provider)?;
        Ok(config)
    }

    /// Load configuration from a file path.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }
}

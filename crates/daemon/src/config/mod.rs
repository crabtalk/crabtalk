//! Daemon configuration loaded from TOML.

pub use ::model::{ProviderDef, ProviderRegistry, validate_providers};
use anyhow::Result;
pub use loader::{DEFAULT_CONFIG, scaffold_config_dir};
pub use runtime::{McpHandler, SystemConfig, mcp::McpServerConfig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(unix)]
pub use wcore::paths::SOCKET_PATH;
pub use wcore::{
    AgentConfig, ManifestConfig, ResolvedManifest, load_agents_dir, load_agents_dirs,
    paths::{AGENTS_DIR, CONFIG_DIR, CONFIG_FILE, SKILLS_DIR},
    resolve_manifests,
};

mod loader;

/// Top-level daemon configuration (`config.toml`).
///
/// System-only: providers, system settings, and env vars for MCP processes.
/// MCPs and agent configs live in manifests (`local/CrabTalk.toml` and
/// `packages/*/*.toml`), loaded via [`resolve_manifests`].
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonConfig {
    /// Provider definitions (`[provider.<name>]`).
    #[serde(default)]
    pub provider: BTreeMap<String, ProviderDef>,
    /// **Deprecated**: MCP configs migrated to `local/CrabTalk.toml`.
    /// Kept for backwards compatibility during migration.
    #[serde(default)]
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// System configuration (tasks + memory).
    #[serde(default)]
    pub system: SystemConfig,
    /// Environment variables passed to all MCP server processes at spawn time.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Disabled resources (providers, MCPs, skills).
    #[serde(default)]
    pub disabled: wcore::config::DisabledItems,
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
        if !config.mcps.is_empty() {
            tracing::warn!("[mcps] in config.toml is deprecated — move to local/CrabTalk.toml");
        }
        validate_providers(&config.provider)?;
        Ok(config)
    }

    /// Load configuration from a file path.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }
}

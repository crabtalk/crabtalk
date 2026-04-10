//! Daemon configuration loaded from TOML.

pub use crate::mcp::{McpHandler, McpServerConfig};
use anyhow::{Result, bail};
pub use loader::{DEFAULT_CONFIG, scaffold_config_dir};
pub use runtime::SystemConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
#[cfg(unix)]
pub use wcore::paths::SOCKET_PATH;
pub use wcore::{
    AgentConfig, ManifestConfig, ProviderDef, ResolvedManifest, load_agents_dir, load_agents_dirs,
    paths::{AGENTS_DIR, CONFIG_DIR, CONFIG_FILE, SKILLS_DIR},
    resolve_manifests,
};

/// Validate provider definitions and reject duplicate model names across providers.
pub fn validate_providers(providers: &BTreeMap<String, ProviderDef>) -> Result<()> {
    let mut seen = HashSet::new();
    for (name, def) in providers {
        def.validate(name).map_err(|e| anyhow::anyhow!(e))?;
        for model in &def.models {
            if !seen.insert(model.clone()) {
                bail!("duplicate model name '{model}' across providers");
            }
        }
    }
    Ok(())
}

mod backfill;
mod loader;

pub use backfill::{backfill_local_agent_ids, migrate_local_agent_prompts};

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

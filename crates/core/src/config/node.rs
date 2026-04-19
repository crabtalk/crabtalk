//! Node configuration loaded from `config.toml`.

use super::system::SystemConfig;
use crate::{McpServerConfig, ProviderDef, config::DisabledItems};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level node configuration (`config.toml`).
///
/// Providers, system settings, and env vars for MCP processes.
/// MCPs and agent configs live in manifests (`local/CrabTalk.toml`
/// and `plugins/*.toml`), loaded via `resolve_manifests`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeConfig {
    /// Provider definitions (`[provider.<name>]`).
    #[serde(default)]
    pub provider: BTreeMap<String, ProviderDef>,
    /// **Deprecated**: MCP configs migrated to `local/CrabTalk.toml`.
    #[serde(default)]
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// System configuration (tasks + memory).
    #[serde(default)]
    pub system: SystemConfig,
    /// Environment variables passed to all MCP server processes.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Disabled resources (providers, MCPs, skills).
    #[serde(default)]
    pub disabled: DisabledItems,
}

impl NodeConfig {
    /// Parse a TOML string into a `NodeConfig`.
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

/// Validate provider definitions and reject duplicate model names.
pub fn validate_providers(providers: &BTreeMap<String, ProviderDef>) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for (name, def) in providers {
        def.validate(name).map_err(|e| anyhow::anyhow!(e))?;
        for model in &def.models {
            if !seen.insert(model.clone()) {
                anyhow::bail!("duplicate model name '{model}' across providers");
            }
        }
    }
    Ok(())
}

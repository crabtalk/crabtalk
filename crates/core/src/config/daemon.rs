//! Daemon configuration loaded from `config.toml`.

use crate::{
    McpServerConfig, ProviderDef,
    config::{
        DisabledItems,
        hooks::{BashConfig, HooksConfig, MemoryConfig},
        system::SystemConfig,
    },
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level daemon configuration (`config.toml`).
///
/// Providers, system settings, and env vars for MCP processes.
/// MCPs and agent configs live in manifests (`local/CrabTalk.toml`
/// and `plugins/*.toml`), loaded via `resolve_manifests`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonConfig {
    /// Provider definitions (`[provider.<name>]`).
    #[serde(default)]
    pub provider: BTreeMap<String, ProviderDef>,
    /// **Deprecated**: MCP configs migrated to `local/CrabTalk.toml`.
    #[serde(default)]
    pub mcps: BTreeMap<String, McpServerConfig>,
    /// System configuration (default agent + task executor).
    #[serde(default)]
    pub system: SystemConfig,
    /// Built-in hook defaults (bash, memory).
    #[serde(default)]
    pub hooks: HooksConfig,
    /// Environment variables passed to all MCP server processes.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Disabled resources (providers, MCPs, skills).
    #[serde(default)]
    pub disabled: DisabledItems,
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
        if let Some(bash) = config.system.legacy_bash.take() {
            tracing::warn!("[system.bash] in config.toml is deprecated — rename to [hooks.bash]");
            if config.hooks.bash == BashConfig::default() {
                config.hooks.bash = bash;
            } else {
                tracing::warn!("[hooks.bash] also set — ignoring deprecated [system.bash]");
            }
        }
        if let Some(memory) = config.system.legacy_memory.take() {
            tracing::warn!(
                "[system.memory] in config.toml is deprecated — rename to [hooks.memory]"
            );
            if config.hooks.memory == MemoryConfig::default() {
                config.hooks.memory = memory;
            } else {
                tracing::warn!("[hooks.memory] also set — ignoring deprecated [system.memory]");
            }
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

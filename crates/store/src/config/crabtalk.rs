//! Top-level configuration loaded from `config.toml`.

use crate::config::{LlmConfig, cache::CacheConfig, env, mcp::McpConfig, system::TasksConfig};
use anyhow::{Context, Result};
use crabllm_core::ProviderConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level configuration (`config.toml`).
///
/// Everything a person writes by hand: the LLM endpoint, the task
/// executor pool, env vars passed to MCP processes. Read once, by
/// whoever starts the daemon — editing it takes a restart.
///
/// Nothing the daemon writes belongs here. What the daemon decides —
/// which agent is default — is store state reached through
/// [`Agents`](crate::interface::Agents), because a field the program
/// rewrites inside a file the user owns is two sources for one value.
/// Per-agent customization lives on each [`AgentConfig`](crate::AgentConfig).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// LLM endpoint (`[llm]`) — one endpoint, reached directly or through
    /// a gateway that routes for us.
    #[serde(default)]
    pub llm: LlmConfig,
    /// LLM endpoints (`[providers]`), keyed by name and routed on the model
    /// each request asks for. A provider that names no `models` has them
    /// read from its catalogue at startup.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    /// Task executor pool configuration (`[tasks]`).
    #[serde(default)]
    pub tasks: TasksConfig,
    /// MCP peer lifetime (`[mcp]`).
    #[serde(default)]
    pub mcp: McpConfig,
    /// Cache budgets (`[cache]`).
    #[serde(default)]
    pub cache: CacheConfig,
    /// Environment variables passed to all MCP server processes.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Config {
    pub fn from_toml(toml_str: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(toml_str)?;
        config.resolve()?;
        Ok(config)
    }

    /// Resolve `${VAR}` in every secret, and reject a file that names the
    /// install's endpoints twice over.
    fn resolve(&mut self) -> Result<()> {
        anyhow::ensure!(
            !(self.llm.is_set() && !self.providers.is_empty()),
            "configure either [llm] or [providers], not both"
        );
        self.llm.api_key = env::interpolate(&self.llm.api_key).context("[llm] api_key")?;
        for (name, provider) in &mut self.providers {
            let Some(key) = &provider.api_key else {
                continue;
            };
            provider.api_key =
                Some(env::interpolate(key).with_context(|| format!("[providers.{name}] api_key"))?);
        }
        Ok(())
    }

    /// Load configuration from a file path. A missing file is an empty
    /// configuration, so an install nobody has configured still starts.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                tracing::info!("configuration from {}", path.display());
                Self::from_toml(&content)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!("no config at {} — using defaults", path.display());
                Ok(Self::default())
            }
            Err(e) => Err(e.into()),
        }
    }
}

//! Top-level configuration loaded from `config.toml`.

use crate::config::{LlmConfig, cache::CacheConfig, mcp::McpConfig, system::TasksConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Name of the file [`Config`] is read from, under the install root.
pub const CONFIG_FILE: &str = "config.toml";

/// Top-level configuration (`config.toml`).
///
/// Everything a person writes by hand: the LLM endpoint, the task
/// executor pool, env vars passed to MCP processes. Read from the file
/// on every reload, so editing it and reloading is the whole workflow.
///
/// Nothing the daemon writes belongs here. What the daemon decides —
/// which agent is default — is store state reached through
/// [`Agents`](crate::interface::Agents), because a field the program
/// rewrites inside a file the user owns is two sources for one value.
/// Per-agent customization lives on each [`AgentConfig`](crate::AgentConfig).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// LLM endpoint (`[llm]`) — single OpenAI-compatible endpoint.
    #[serde(default)]
    pub llm: LlmConfig,
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
        Ok(toml::from_str(toml_str)?)
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
                tracing::warn!("no {CONFIG_FILE} at {} — using defaults", path.display());
                Ok(Self::default())
            }
            Err(e) => Err(e.into()),
        }
    }
}

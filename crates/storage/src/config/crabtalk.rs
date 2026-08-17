//! Top-level configuration loaded from `config.toml`.

use crate::AgentId;
use crate::config::{LlmConfig, mcp::McpConfig, system::TasksConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Name of the file [`Config`] is read from, under the install root.
pub const CONFIG_FILE: &str = "config.toml";

/// Top-level configuration (`config.toml`).
///
/// Holds immutable per-install settings: the LLM endpoint, task executor
/// pool, and env vars passed to MCP processes. Mutable runtime records
/// (MCPs, agents) live in [`crate::storage::Storage`]. Per-agent
/// customization (hooks, etc.) lives directly on each
/// [`crate::AgentConfig`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// The agent a surface talks to when it names none. Seeded at
    /// scaffold with the zero ULID; repoint it at any other agent's id.
    #[serde(default)]
    pub default_agent: AgentId,
    /// LLM endpoint (`[llm]`) — single OpenAI-compatible endpoint.
    #[serde(default)]
    pub llm: LlmConfig,
    /// Task executor pool configuration (`[tasks]`).
    #[serde(default)]
    pub tasks: TasksConfig,
    /// MCP peer lifetime (`[mcp]`).
    #[serde(default)]
    pub mcp: McpConfig,
    /// Environment variables passed to all MCP server processes.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Config {
    pub fn from_toml(toml_str: &str) -> Result<Self> {
        Ok(toml::from_str(toml_str)?)
    }

    /// Load configuration from a file path.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }
}

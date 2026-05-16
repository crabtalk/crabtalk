//! Top-level configuration loaded from `config.toml`.

use crate::config::{LlmConfig, system::TasksConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level configuration (`config.toml`).
///
/// Holds immutable per-install settings: the LLM endpoint, task executor
/// pool, and env vars passed to MCP processes. Mutable runtime records
/// (MCPs, agents) live in [`crate::storage::Storage`]. Per-agent
/// customization (hooks, etc.) lives directly on each
/// [`crate::AgentConfig`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// LLM endpoint (`[llm]`) — single OpenAI-compatible endpoint.
    #[serde(default)]
    pub llm: LlmConfig,
    /// Task executor pool configuration (`[tasks]`).
    #[serde(default)]
    pub tasks: TasksConfig,
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

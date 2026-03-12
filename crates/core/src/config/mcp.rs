//! MCP server configuration.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpServerConfig {
    /// Server name. If empty, the name will be the command.
    pub name: CompactString,
    /// Command to spawn.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Auto-restart on failure.
    pub auto_restart: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: CompactString::default(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            auto_restart: true,
        }
    }
}

//! MCP server configuration.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpServerConfig {
    /// Server name. If empty, the name will be the command.
    pub name: String,
    /// Command to spawn (stdio transport).
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Auto-restart on failure.
    pub auto_restart: bool,
    /// HTTP URL for streamable HTTP transport. When set, the daemon connects
    /// via HTTP instead of spawning a child process.
    pub url: Option<String>,
    /// Full `Authorization` header value to send on every HTTP-transport
    /// request, e.g. `"Bearer eyJ..."`. Caller picks the scheme. Ignored
    /// for stdio transports.
    pub auth: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            auto_restart: true,
            url: None,
            auth: None,
        }
    }
}

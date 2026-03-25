//! Crabtalk hub manifest.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wcore::Setup;

/// Crabtalk resource manifest.
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    /// the package manifest
    pub package: Package,

    /// MCP server configs
    #[serde(default)]
    pub mcps: BTreeMap<String, McpResource>,

    /// Agent resources
    #[serde(default)]
    pub agents: BTreeMap<String, AgentResource>,

    /// Command service metadata
    #[serde(default)]
    pub commands: BTreeMap<String, CommandConfig>,
}

/// The package manifest.
#[derive(Serialize, Deserialize)]
pub struct Package {
    /// Package name.
    pub name: String,
    /// Package description (for hub display).
    #[serde(default)]
    pub description: String,
    /// Logo URL (for hub display).
    #[serde(default)]
    pub logo: String,
    /// Source repository URL.
    #[serde(default)]
    pub repository: String,
    /// Branch to clone (defaults to the repo's default branch).
    #[serde(default)]
    pub branch: Option<String>,
    /// Searchable keywords (for hub discovery).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Setup configuration (run after install).
    #[serde(default)]
    pub setup: Option<Setup>,
}

/// An MCP server resource in a hub manifest.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct McpResource {
    /// Server name. If empty, defaults to the command.
    pub name: String,
    /// Command to spawn (stdio transport).
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Auto-restart on failure.
    pub auto_restart: bool,
    /// HTTP URL for streamable HTTP transport.
    pub url: Option<String>,
    /// Whether this server requires OAuth authentication.
    pub auth: bool,
}

impl Default for McpResource {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            auto_restart: true,
            url: None,
            auth: false,
        }
    }
}

impl McpResource {
    /// Convert to the runtime MCP config.
    pub fn to_server_config(&self) -> wcore::McpServerConfig {
        wcore::McpServerConfig {
            name: self.name.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: self.env.clone(),
            auto_restart: self.auto_restart,
            url: self.url.clone(),
            auth: self.auth,
        }
    }
}

/// An agent resource — discovered by convention from `agents/*.md`.
#[derive(Serialize, Deserialize)]
pub struct AgentResource {
    /// Agent description
    #[serde(default)]
    pub description: String,
    /// Path to the prompt `.md` file (legacy, optional)
    #[serde(default)]
    pub prompt: String,
    /// Skill keys from `[skills.*]` in the same manifest
    #[serde(default)]
    pub skills: Vec<String>,
    /// Model override for this agent
    #[serde(default)]
    pub model: Option<String>,
    /// Whether to enable thinking/reasoning mode
    #[serde(default)]
    pub thinking: bool,
    /// MCP server names this agent can access
    #[serde(default)]
    pub mcps: Vec<String>,
}

/// Command service metadata for hub registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandConfig {
    /// Human-readable description.
    pub description: String,
    /// Crate name on crates.io (installed via `cargo install`).
    #[serde(rename = "crate")]
    pub krate: String,
}

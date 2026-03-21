//! Crabtalk hub manifest.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wcore::CommandConfig;

/// Crabtalk resource manifest.
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    /// the package manifest
    pub package: Package,

    /// MCP server configs
    #[serde(default)]
    pub mcps: BTreeMap<String, McpResource>,

    /// Skill resources
    #[serde(default)]
    pub skills: BTreeMap<String, SkillResource>,

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
    /// Searchable keywords (for hub discovery).
    #[serde(default)]
    pub keywords: Vec<String>,
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
    /// Optional setup command to run after install.
    pub setup: Option<SetupConfig>,
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
            setup: None,
        }
    }
}

impl McpResource {
    /// Convert to the runtime MCP config (without setup).
    pub fn to_server_config(&self) -> wcore::McpServerConfig {
        wcore::McpServerConfig {
            name: self.name.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: self.env.clone(),
            auto_restart: self.auto_restart,
            url: self.url.clone(),
        }
    }
}

/// A setup command to run after install.
#[derive(Serialize, Deserialize)]
pub struct SetupConfig {
    /// Shell command to execute.
    pub run: String,
    /// Human-readable message shown before running.
    pub message: String,
}

/// A skill resource.
#[derive(Serialize, Deserialize)]
pub struct SkillResource {
    /// Skill name (defaults to map key if empty)
    #[serde(default)]
    pub name: String,
    /// Skill description
    pub description: String,
    /// Path within the repo to the skill directory
    pub path: String,
    /// Optional setup command to run after install.
    #[serde(default)]
    pub setup: Option<SetupConfig>,
}

/// An agent resource — system prompt + skill bundle.
#[derive(Serialize, Deserialize)]
pub struct AgentResource {
    /// Agent description
    pub description: String,
    /// Path to the prompt `.md` file in the hub repo (relative to scope dir)
    pub prompt: String,
    /// Skill keys from `[skills.*]` in the same manifest to auto-install
    #[serde(default)]
    pub skills: Vec<String>,
}

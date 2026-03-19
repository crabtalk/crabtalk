//! Crabtalk hub manifest.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wcore::{McpServerConfig, ServiceConfig};

/// Crabtalk resource manifest.
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    /// the package manifest
    pub package: Package,

    /// MCP server configs
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,

    /// Skill resources
    #[serde(default)]
    pub skills: BTreeMap<String, SkillResource>,

    /// Service configs
    #[serde(default)]
    pub services: BTreeMap<String, ServiceConfig>,

    /// Agent resources
    #[serde(default)]
    pub agents: BTreeMap<String, AgentResource>,
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

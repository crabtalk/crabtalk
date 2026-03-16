//! walrus hub manifest

use crate::{hook::mcp::McpServerConfig, service::ServiceConfig};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Walrus resource manifest
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    /// the package manifest
    pub package: Package,

    /// MCP server configs
    #[serde(default)]
    pub mcp_servers: BTreeMap<CompactString, McpServerConfig>,

    /// Skill resources
    #[serde(default)]
    pub skills: BTreeMap<CompactString, SkillResource>,

    /// WHS service configs
    #[serde(default)]
    pub services: BTreeMap<CompactString, ServiceConfig>,

    /// Agent resources
    #[serde(default)]
    pub agents: BTreeMap<CompactString, AgentResource>,
}

/// The package manifest
#[derive(Serialize, Deserialize)]
pub struct Package {
    /// Package name.
    pub name: CompactString,
    /// Package description (for hub display).
    #[serde(default)]
    pub description: CompactString,
    /// Logo URL (for hub display).
    #[serde(default)]
    pub logo: CompactString,
    /// Source repository URL.
    #[serde(default)]
    pub repository: CompactString,
    /// Searchable keywords (for hub discovery).
    #[serde(default)]
    pub keywords: Vec<CompactString>,
}

/// A skill resource
#[derive(Serialize, Deserialize)]
pub struct SkillResource {
    /// Skill name (defaults to map key if empty)
    #[serde(default)]
    pub name: CompactString,
    /// Skill description
    pub description: CompactString,
    /// Path within the repo to the skill directory
    pub path: CompactString,
}

/// An agent resource — system prompt + skill bundle.
#[derive(Serialize, Deserialize)]
pub struct AgentResource {
    /// Agent description
    pub description: CompactString,
    /// Path to the prompt `.md` file in the hub repo (relative to scope dir)
    pub prompt: CompactString,
    /// Skill keys from `[skills.*]` in the same manifest to auto-install
    #[serde(default)]
    pub skills: Vec<CompactString>,
}

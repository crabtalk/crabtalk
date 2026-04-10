//! Skill domain type.

use std::collections::BTreeMap;

/// A named unit of agent behavior (agentskills.io format).
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub allowed_tools: Vec<String>,
    pub body: String,
}

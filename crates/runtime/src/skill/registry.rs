//! Crabtalk skill registry — skill storage and lookup.

use std::collections::BTreeMap;

/// A registry of loaded skills.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    pub skills: Vec<Skill>,
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a skill to the registry.
    pub fn add(&mut self, skill: Skill) {
        self.skills.push(skill);
    }

    /// Add or replace a skill by name.
    pub fn upsert(&mut self, skill: Skill) {
        self.skills.retain(|s| s.name != skill.name);
        self.skills.push(skill);
    }

    /// Whether a skill with the given name is already registered.
    pub fn contains(&self, name: &str) -> bool {
        self.skills.iter().any(|s| s.name == name)
    }
}

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

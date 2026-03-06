//! Walrus skill registry — skill storage and lookup.

use compact_str::CompactString;
use std::collections::BTreeMap;

/// A registry of loaded skills.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
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

    /// Get all loaded skills.
    pub fn skills(&self) -> Vec<&Skill> {
        self.skills.iter().collect()
    }

    /// Number of loaded skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Whether the registry has no skills.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

/// A named unit of agent behavior (agentskills.io format).
///
/// Pure data struct — parsing logic lives in the [`loader`] module.
/// Fields mirror the agentskills.io specification.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Skill identifier (lowercase, hyphens, 1-64 chars).
    pub name: CompactString,
    /// Human-readable description (1-1024 chars).
    pub description: String,
    /// License name or reference to a bundled license file.
    pub license: Option<CompactString>,
    /// Compatibility constraints (e.g. "Requires git, docker").
    pub compatibility: Option<CompactString>,
    /// Arbitrary key-value metadata map.
    pub metadata: BTreeMap<CompactString, String>,
    /// Tool names this skill is pre-approved to use (experimental).
    pub allowed_tools: Vec<CompactString>,
    /// Skill body (Markdown instructions).
    pub body: String,
}

//! Skill repository trait and domain type.
//!
//! [`SkillRepo`] abstracts skill discovery and loading. Implementations
//! own multi-root composition and the SKILL.md parsing pipeline. The
//! trait is read-only — skill creation is out of scope (users author
//! SKILL.md files on disk or install packages).

use anyhow::Result;
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

/// Persistence backend for skill discovery.
///
/// Implementations own multi-root composition, SKILL.md parsing, and
/// filtering of hidden/disabled skills. The trait speaks domain types
/// only — no path manipulation, no byte buffers.
pub trait SkillRepo: Send + Sync + 'static {
    /// List all available skills.
    fn list(&self) -> Result<Vec<Skill>>;

    /// Load a specific skill by name. Returns `None` if not found.
    fn load(&self, name: &str) -> Result<Option<Skill>>;
}

//! The SKILL.md standard (agentskills.io).
//!
//! A skill is a directory holding a `SKILL.md` — YAML frontmatter naming it,
//! Markdown saying what it does. That is the whole format, and this crate is
//! the whole of it: the type, the parse, and the rules for finding one on
//! disk. Nothing here knows what a runtime or an agent is.

use std::collections::BTreeMap;

pub mod discover;

mod md;

/// A named unit of agent behavior.
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

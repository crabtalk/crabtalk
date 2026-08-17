//! Skill domain types.
//!
//! [`Skill`] itself is the SKILL.md standard, owned by the `skill`
//! crate. What lives here is the split between a skill's identity and
//! its body: `Skill::body` is the entire markdown, so any listing must
//! carry [`SkillSummary`] instead — a store holding a large catalog
//! cannot answer "what skills are there" by reading all of them.

pub use ::skill::Skill;

/// A skill's identity, without its body.
///
/// What a listing renders and what the index stores. The body is a KV
/// read, made only for the skill an agent actually invokes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
}

impl From<&Skill> for SkillSummary {
    fn from(skill: &Skill) -> Self {
        Self {
            name: skill.name.clone(),
            description: skill.description.clone(),
        }
    }
}

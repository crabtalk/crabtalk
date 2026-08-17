//! Installed skills, and what one is.
//!
//! [`Skill`] itself is the SKILL.md standard, owned by the `skill`
//! crate. What is added here is the split between a skill's identity and
//! its body: `Skill::body` is the entire markdown, so any listing must
//! carry [`SkillSummary`] instead — a store holding a large catalogue
//! cannot answer "what skills are there" by reading all of them.

use crate::kv::{Column, KVStorage};
use anyhow::Result;
use std::{future::Future, str::FromStr};

pub use ::skill::Skill;

/// A skill's identity, without its body.
///
/// What a listing renders and what the store keeps beside the markdown.
/// The body is a second read, made only for the skill an agent actually
/// invokes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Installed skills.
///
/// A skill's identity and its body are separate keys, so "a listing
/// never reads markdown" is a property of the layout rather than a rule
/// each backend has to remember.
pub trait Skills: KVStorage {
    fn list_skills(
        &self,
        limit: usize,
        offset: usize,
    ) -> impl Future<Output = Result<Vec<SkillSummary>>> + Send {
        async move {
            let keys = self
                .scan_keys(Column::Skill, &self.prefix(&["skill", "meta"]))
                .await?;
            let mut out = Vec::new();
            for key in keys.iter().skip(offset).take(limit) {
                if let Some(summary) = self.get_json(Column::Skill, key).await? {
                    out.push(summary);
                }
            }
            Ok(out)
        }
    }

    fn load_skill(&self, name: &str) -> impl Future<Output = Result<Option<Skill>>> + Send {
        async move {
            let key = self.key(&["skill", "body", name]);
            let Some(bytes) = self.get(Column::Skill, &key).await? else {
                return Ok(None);
            };
            Ok(Some(Skill::from_str(&String::from_utf8(bytes)?)?))
        }
    }

    /// Store a skill from its `SKILL.md`. The markdown is what is kept —
    /// it is the standard's own format, so it round-trips exactly and the
    /// name cannot disagree with the frontmatter it came from.
    fn put_skill(&self, markdown: &str) -> impl Future<Output = Result<SkillSummary>> + Send {
        async move {
            let skill = Skill::from_str(markdown)?;
            let summary = SkillSummary::from(&skill);
            self.put(
                Column::Skill,
                &self.key(&["skill", "body", &skill.name]),
                markdown.as_bytes(),
            )
            .await?;
            self.put_json(
                Column::Skill,
                &self.key(&["skill", "meta", &skill.name]),
                &summary,
            )
            .await?;
            Ok(summary)
        }
    }

    fn remove_skill(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        async move {
            let had_meta = self
                .delete(Column::Skill, &self.key(&["skill", "meta", name]))
                .await?;
            let had_body = self
                .delete(Column::Skill, &self.key(&["skill", "body", name]))
                .await?;
            Ok(had_meta || had_body)
        }
    }
}

impl<T: KVStorage> Skills for T {}

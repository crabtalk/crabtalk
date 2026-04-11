//! Skill markdown parsing.
//!
//! [`parse_skill_md`] parses a SKILL.md file (YAML frontmatter +
//! Markdown body) into a [`Skill`]. Used by daemon's `FsSkillRepo`
//! when loading skills from disk.

use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use wcore::{repos::Skill, utils::split_yaml_frontmatter};

/// Accept both `"a, b, c"` (string) and `["a", "b", "c"]` (sequence) for tool lists.
fn string_or_vec<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }
    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::Vec(v) => Ok(v),
        StringOrVec::String(s) => Ok(s
            .split([',', ' '])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()),
    }
}

/// YAML frontmatter deserialization target for SKILL.md files.
#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default, rename = "allowed-tools", deserialize_with = "string_or_vec")]
    allowed_tools: Vec<String>,
}

/// Parse a SKILL.md file (YAML frontmatter + Markdown body) into a [`Skill`].
pub fn parse_skill_md(content: &str) -> anyhow::Result<Skill> {
    let (frontmatter, body) = split_yaml_frontmatter(content)?;
    let fm: SkillFrontmatter = serde_yml::from_str(frontmatter)?;

    Ok(Skill {
        name: fm.name,
        description: fm.description,
        license: fm.license,
        compatibility: fm.compatibility,
        metadata: fm.metadata,
        allowed_tools: fm.allowed_tools,
        body: body.to_owned(),
    })
}

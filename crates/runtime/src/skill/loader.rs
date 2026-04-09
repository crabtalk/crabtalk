//! Skill markdown loading.

use crate::skill::{Skill, SkillRegistry};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use wcore::{Storage, utils::split_yaml_frontmatter};

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

    let metadata = fm.metadata;

    Ok(Skill {
        name: fm.name,
        description: fm.description,
        license: fm.license,
        compatibility: fm.compatibility,
        metadata,
        allowed_tools: fm.allowed_tools,
        body: body.to_owned(),
    })
}

/// Load all skills the given [`Storage`] backend can see. The scan looks
/// for any key ending in `SKILL.md`, interpreting the segment before the
/// final slash as the skill's directory (a bundle root). Hidden path
/// components (names starting with `.`) are skipped.
pub fn load_skills_from_storage(storage: &dyn Storage) -> anyhow::Result<SkillRegistry> {
    let mut registry = SkillRegistry::new();
    let keys = storage.list("")?;
    for key in keys {
        if !is_skill_manifest_key(&key) {
            continue;
        }
        let Some(bytes) = storage.get(&key)? else {
            continue;
        };
        let Ok(content) = std::str::from_utf8(&bytes) else {
            tracing::warn!("skill manifest {key} is not valid UTF-8, skipping");
            continue;
        };
        match parse_skill_md(content) {
            Ok(skill) => registry.add(skill),
            Err(e) => tracing::warn!("failed to parse {key}: {e}"),
        }
    }
    Ok(registry)
}

/// Is `key` a candidate SKILL.md manifest? Must end in `SKILL.md` and
/// have no dot-prefixed path component (matching the old fs scanner
/// which skipped hidden directories).
fn is_skill_manifest_key(key: &str) -> bool {
    if !key.ends_with("SKILL.md") {
        return false;
    }
    !key.split('/').any(|segment| segment.starts_with('.'))
}

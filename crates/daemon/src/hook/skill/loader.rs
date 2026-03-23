//! Skill markdown loading.
//!
//! Parses `SKILL.md` files (YAML frontmatter + Markdown body) from skill
//! directories and builds a [`SkillRegistry`].

use crate::hook::skill::{Skill, SkillRegistry};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
use wcore::utils::split_yaml_frontmatter;

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
    #[serde(default, rename = "allowed-tools")]
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

/// Load skills by searching for `SKILL.md` files in subdirectories.
///
/// Ignores any `SKILL.md` at the root — that's meta content, not a skill.
/// Once a `SKILL.md` is found in a subdirectory, that directory is a skill
/// and we don't recurse deeper. Skips hidden directories (starting with `.`).
pub fn load_skills_dir(path: impl AsRef<Path>) -> anyhow::Result<SkillRegistry> {
    let path = path.as_ref();
    let mut registry = SkillRegistry::new();
    scan_skills(path, &mut registry)?;
    Ok(registry)
}

fn scan_skills(dir: &Path, registry: &mut SkillRegistry) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = entry?;
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        let skill_file = entry_path.join("SKILL.md");
        if skill_file.exists() {
            // Found a skill — load it and don't recurse deeper.
            let content = std::fs::read_to_string(&skill_file)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", skill_file.display()))?;
            match parse_skill_md(&content) {
                Ok(skill) => registry.add(skill),
                Err(e) => {
                    tracing::warn!("failed to parse {}: {e}", skill_file.display());
                }
            }
        } else {
            // No SKILL.md here — recurse into subdirs.
            scan_skills(&entry_path, registry)?;
        }
    }

    Ok(())
}

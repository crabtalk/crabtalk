//! Skill registry — loads, indexes, and matches skills.
//!
//! Skills are directories containing a `SKILL.md` file with YAML frontmatter
//! (agentskills.io format). The [`SkillRegistry`] loads them from a directory,
//! builds tag/trigger indices, and returns ranked matches.

use agent::{Skill, SkillTier};
use compact_str::CompactString;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// An indexed skill with its tier and priority (extracted from metadata).
#[derive(Debug, Clone)]
struct IndexedSkill {
    skill: Skill,
    tier: SkillTier,
    priority: u8,
}

/// YAML frontmatter deserialization target.
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
    allowed_tools: Option<String>,
}

/// A registry of loaded skills with tag and trigger indices.
#[derive(Debug)]
pub struct SkillRegistry {
    skills: Vec<IndexedSkill>,
    tag_index: BTreeMap<CompactString, Vec<usize>>,
    trigger_index: BTreeMap<CompactString, Vec<usize>>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            tag_index: BTreeMap::new(),
            trigger_index: BTreeMap::new(),
        }
    }

    /// Load skills from a directory. Each subdirectory should contain a `SKILL.md`.
    /// The given tier is assigned to all loaded skills.
    pub fn load_dir(path: impl AsRef<Path>, tier: SkillTier) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let mut registry = Self::new();

        let entries = std::fs::read_dir(path).map_err(|e| {
            anyhow::anyhow!("failed to read skill directory {}: {e}", path.display())
        })?;

        for entry in entries {
            let entry = entry?;
            let entry_path = entry.path();
            if !entry_path.is_dir() {
                continue;
            }

            let skill_file = entry_path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }

            let content = std::fs::read_to_string(&skill_file)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", skill_file.display()))?;

            let skill = parse_skill_md(&content)?;
            registry.add(skill, tier);
        }

        Ok(registry)
    }

    /// Add a skill to the registry with the given tier.
    pub fn add(&mut self, skill: Skill, tier: SkillTier) {
        let priority = skill
            .metadata
            .get("priority")
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(0);

        let idx = self.skills.len();

        // Index tags from metadata["tags"] (comma-separated).
        if let Some(tags) = skill.metadata.get("tags") {
            for tag in tags.split(',') {
                let tag = tag.trim();
                if !tag.is_empty() {
                    self.tag_index
                        .entry(CompactString::from(tag))
                        .or_default()
                        .push(idx);
                }
            }
        }

        // Index triggers from metadata["triggers"] (comma-separated).
        if let Some(triggers) = skill.metadata.get("triggers") {
            for trigger in triggers.split(',') {
                let trigger = trigger.trim().to_lowercase();
                if !trigger.is_empty() {
                    self.trigger_index
                        .entry(CompactString::from(trigger))
                        .or_default()
                        .push(idx);
                }
            }
        }

        self.skills.push(IndexedSkill {
            skill,
            tier,
            priority,
        });
    }

    /// Find skills matching any of the given tags, sorted by tier (desc) then priority (desc).
    pub fn find_by_tags(&self, tags: &[CompactString]) -> Vec<&Skill> {
        let mut indices: Vec<usize> = tags
            .iter()
            .filter_map(|tag| self.tag_index.get(tag))
            .flatten()
            .copied()
            .collect();

        indices.sort_unstable();
        indices.dedup();

        // Sort by tier desc, then priority desc.
        indices.sort_by(|&a, &b| {
            let sa = &self.skills[a];
            let sb = &self.skills[b];
            sb.tier
                .cmp(&sa.tier)
                .then_with(|| sb.priority.cmp(&sa.priority))
        });

        indices.iter().map(|&i| &self.skills[i].skill).collect()
    }

    /// Find skills whose trigger keywords match the query (case-insensitive).
    pub fn find_by_trigger(&self, query: &str) -> Vec<&Skill> {
        let query_lower = query.to_lowercase();
        let mut indices: Vec<usize> = self
            .trigger_index
            .iter()
            .filter(|(keyword, _)| query_lower.contains(keyword.as_str()))
            .flat_map(|(_, idxs)| idxs.iter().copied())
            .collect();

        indices.sort_unstable();
        indices.dedup();

        // Sort by tier desc, then priority desc.
        indices.sort_by(|&a, &b| {
            let sa = &self.skills[a];
            let sb = &self.skills[b];
            sb.tier
                .cmp(&sa.tier)
                .then_with(|| sb.priority.cmp(&sa.priority))
        });

        indices.iter().map(|&i| &self.skills[i].skill).collect()
    }

    /// Get all loaded skills.
    pub fn skills(&self) -> Vec<&Skill> {
        self.skills.iter().map(|s| &s.skill).collect()
    }

    /// Number of loaded skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

/// Parse a SKILL.md file (YAML frontmatter + Markdown body) into a Skill.
pub fn parse_skill_md(content: &str) -> anyhow::Result<Skill> {
    let (frontmatter, body) = split_yaml_frontmatter(content)?;
    let fm: SkillFrontmatter = serde_yaml::from_str(frontmatter)?;

    let allowed_tools = fm
        .allowed_tools
        .map(|s| {
            s.split_whitespace()
                .map(CompactString::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let metadata = fm
        .metadata
        .into_iter()
        .map(|(k, v)| (CompactString::from(k), v))
        .collect();

    Ok(Skill {
        name: CompactString::from(fm.name),
        description: fm.description,
        license: fm.license.map(CompactString::from),
        compatibility: fm.compatibility.map(CompactString::from),
        metadata,
        allowed_tools,
        body: body.to_owned(),
    })
}

/// Split YAML frontmatter from the body. Frontmatter is delimited by `---`.
///
/// Handles CRLF line endings and trailing whitespace on delimiter lines.
fn split_yaml_frontmatter(content: &str) -> anyhow::Result<(&str, &str)> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        anyhow::bail!("missing YAML frontmatter delimiter (---)");
    }

    // Skip opening delimiter and its trailing newline.
    let after_first = content[3..].trim_start_matches(['\n', '\r']);

    // Scan line-by-line for the closing `---` delimiter.
    let mut pos = 0;
    for line in after_first.lines() {
        if line.trim() == "---" {
            let frontmatter = &after_first[..pos].trim_end();
            let body_start = pos + line.len();
            // Skip the newline after `---` if present.
            let body = after_first[body_start..].trim_start_matches(['\n', '\r']);
            return Ok((frontmatter, body));
        }
        pos += line.len() + 1; // +1 for the newline consumed by lines()
    }

    anyhow::bail!("missing closing YAML frontmatter delimiter (---)")
}

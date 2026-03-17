//! Tool dispatch and schema registration for skill tools.

use crate::hook::{DaemonHook, skill::loader};
use serde::{Deserialize, Serialize};
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct SearchSkill {
    /// Keyword to match skill names and descriptions. Leave empty to list all.
    pub query: String,
}

impl ToolDescription for SearchSkill {
    const DESCRIPTION: &'static str =
        "Search available skills by keyword. Returns name and description only.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct LoadSkill {
    /// Skill name
    pub name: String,
}

impl ToolDescription for LoadSkill {
    const DESCRIPTION: &'static str = "Load a skill by name. Returns its instructions and the skill directory path for resolving relative file references.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct SaveSkill {
    /// Skill name (lowercase alphanumeric with hyphens, 1-64 chars).
    pub name: String,
    /// One-line description of what the skill does.
    pub description: String,
    /// The skill body — Markdown instructions for agents.
    pub body: String,
    /// Space-separated tool names this skill is allowed to use (optional).
    #[serde(default)]
    pub allowed_tools: Option<String>,
}

impl ToolDescription for SaveSkill {
    const DESCRIPTION: &'static str = "Save a skill as a SKILL.md file. Creates the skill directory and writes the file with YAML frontmatter.";
}

/// Serialization target for SKILL.md YAML frontmatter (safe from injection).
#[derive(Serialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(rename = "allowed-tools", skip_serializing_if = "Option::is_none")]
    allowed_tools: Option<String>,
}

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        SearchSkill::as_tool(),
        LoadSkill::as_tool(),
        SaveSkill::as_tool(),
    ]
}

impl DaemonHook {
    pub(crate) async fn dispatch_search_skill(&self, args: &str, agent: &str) -> String {
        let input: SearchSkill = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let query = input.query.to_lowercase();
        // Get agent's allowed skills for filtering.
        let allowed = self.scopes.get(agent).map(|s| &s.skills);
        let registry = self.skills.registry.lock().await;
        let matches: Vec<String> = registry
            .skills()
            .into_iter()
            .filter(|s| {
                // Filter by agent's skills scope if non-empty.
                if let Some(allowed) = allowed
                    && !allowed.is_empty()
                    && !allowed.iter().any(|a| a == s.name.as_str())
                {
                    return false;
                }
                s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
            })
            .map(|s| format!("{}: {}", s.name, s.description))
            .collect();
        if matches.is_empty() {
            "no skills found".to_owned()
        } else {
            matches.join("\n")
        }
    }

    pub(crate) async fn dispatch_save_skill(&self, args: &str) -> String {
        let input: SaveSkill = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let name = &input.name;
        if name.is_empty() || name.len() > 64 {
            return "skill name must be 1-64 characters".to_owned();
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c.is_ascii_digit())
        {
            return "skill name must be lowercase alphanumeric with hyphens".to_owned();
        }

        // Serialize frontmatter via serde_yaml to prevent YAML injection.
        let fm = SkillFrontmatter {
            name: name.clone(),
            description: input.description,
            allowed_tools: input.allowed_tools,
        };
        let yaml = match serde_yaml::to_string(&fm) {
            Ok(y) => y,
            Err(e) => return format!("failed to serialize frontmatter: {e}"),
        };
        let content = format!("---\n{yaml}---\n\n{}", input.body);

        // Validate by round-tripping through the standard parser.
        let skill = match loader::parse_skill_md(&content) {
            Ok(s) => s,
            Err(e) => return format!("generated skill is invalid: {e}"),
        };

        // Write to skills directory.
        let skill_dir = self.skills.skills_dir.join(name);
        if let Err(e) = tokio::fs::create_dir_all(&skill_dir).await {
            return format!("failed to create skill directory: {e}");
        }
        let skill_file = skill_dir.join("SKILL.md");
        match tokio::fs::write(&skill_file, &content).await {
            Ok(()) => {
                self.skills.registry.lock().await.upsert(skill);
                format!("skill saved: {}", skill_file.display())
            }
            Err(e) => format!("failed to write skill: {e}"),
        }
    }

    pub(crate) async fn dispatch_load_skill(&self, args: &str, agent: &str) -> String {
        let input: LoadSkill = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let name = &input.name;
        // Enforce skill scope.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.skills.is_empty()
            && !scope.skills.iter().any(|s| s == name)
        {
            return format!("skill not available: {name}");
        }
        // Guard against path traversal in the skill name.
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return format!("invalid skill name: {name}");
        }
        let skill_dir = self.skills.skills_dir.join(name);
        let skill_file = skill_dir.join("SKILL.md");
        let content = match tokio::fs::read_to_string(&skill_file).await {
            Ok(c) => c,
            Err(_) => return format!("skill not found: {name}"),
        };
        let skill = match loader::parse_skill_md(&content) {
            Ok(s) => s,
            Err(e) => return format!("failed to parse skill: {e}"),
        };
        let body = skill.body.clone();
        self.skills.registry.lock().await.add(skill);
        let dir_path = skill_dir.display();
        format!("{body}\n\nSkill directory: {dir_path}")
    }
}

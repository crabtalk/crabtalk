//! Tool dispatch and schema registration for the skill tool.

use crate::{Env, host::Host, skill::loader};
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Skill {
    /// Skill name to load. If no exact match, returns fuzzy matches.
    /// Leave empty to list all available skills.
    pub name: String,
}

impl ToolDescription for Skill {
    const DESCRIPTION: &'static str = "Load a skill by name. Returns its instructions on exact match, or lists matching skills otherwise.";
}

pub fn tools() -> Vec<Tool> {
    vec![Skill::as_tool()]
}

impl<H: Host> Env<H> {
    pub async fn dispatch_skill(&self, args: &str, agent: &str) -> Result<String, String> {
        let input: Skill =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;
        let name = &input.name;

        // Enforce skill scope.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.skills.is_empty()
            && !scope.skills.iter().any(|s| s == name)
        {
            return Err(format!("skill not available: {name}"));
        }

        // Guard against path traversal.
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err(format!("invalid skill name: {name}"));
        }

        // Try exact load from each skill directory.
        if !name.is_empty() {
            for dir in &self.skills.skill_dirs {
                let skill_dir = dir.join(name);
                let skill_file = skill_dir.join("SKILL.md");
                if let Ok(content) = tokio::fs::read_to_string(&skill_file).await {
                    return match loader::parse_skill_md(&content) {
                        Ok(skill) => {
                            let body = skill.body.clone();
                            self.skills.registry.lock().await.upsert(skill);
                            let dir_path = skill_dir.display();
                            Ok(format!("{body}\n\nSkill directory: {dir_path}"))
                        }
                        Err(e) => Err(format!("failed to parse skill: {e}")),
                    };
                }
            }
        }

        // No exact match — fuzzy search / list all.
        let query = name.to_lowercase();
        let allowed = self.scopes.get(agent).map(|s| &s.skills);
        let registry = self.skills.registry.lock().await;
        let matches: Vec<String> = registry
            .skills
            .iter()
            .filter(|s| {
                if let Some(allowed) = allowed
                    && !allowed.is_empty()
                    && !allowed.iter().any(|a| a == s.name.as_str())
                {
                    return false;
                }
                query.is_empty()
                    || s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
            })
            .map(|s| format!("{}: {}", s.name, s.description))
            .collect();

        // Empty discovery is not a failure — the caller asked "what matches?"
        // and got "nothing". Return Ok so the UI doesn't flag it as an error.
        if matches.is_empty() {
            Ok("no skills found".to_owned())
        } else {
            Ok(matches.join("\n"))
        }
    }
}

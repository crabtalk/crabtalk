//! Tool dispatch and schema registration for the skill tool.

use crate::{Env, host::Host};
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
    repos::{Repos, SkillRepo},
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SkillTool {
    /// Skill name to load. If no exact match, returns fuzzy matches.
    /// Leave empty to list all available skills.
    pub name: String,
}

impl ToolDescription for SkillTool {
    const DESCRIPTION: &'static str = "Load a skill by name. Returns its instructions on exact match, or lists matching skills otherwise.";
}

pub fn tools() -> Vec<Tool> {
    vec![SkillTool::as_tool()]
}

impl<H: Host, R: Repos> Env<H, R> {
    pub async fn dispatch_skill(&self, args: &str, agent: &str) -> Result<String, String> {
        let input: SkillTool =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;
        let name = &input.name;

        // Enforce skill scope.
        {
            let scopes = self.scopes.read().expect("scopes lock poisoned");
            if let Some(scope) = scopes.get(agent)
                && !scope.skills.is_empty()
                && !scope.skills.iter().any(|s| s == name)
            {
                return Err(format!("skill not available: {name}"));
            }
        }

        // Guard against path traversal.
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err(format!("invalid skill name: {name}"));
        }

        // Try exact load from the repo.
        if !name.is_empty() {
            match self.repos.skills().load(name) {
                Ok(Some(skill)) => return Ok(skill.body),
                Ok(None) => {} // fall through to fuzzy search
                Err(e) => return Err(format!("failed to load skill: {e}")),
            }
        }

        // No exact match — fuzzy search / list all.
        let query = name.to_lowercase();
        let allowed: Vec<String> = self
            .scopes
            .read()
            .expect("scopes lock poisoned")
            .get(agent)
            .map(|s| s.skills.clone())
            .unwrap_or_default();

        let skills = self.repos.skills().list().unwrap_or_default();
        let matches: Vec<String> = skills
            .iter()
            .filter(|s| {
                if !allowed.is_empty() && !allowed.iter().any(|a| a == s.name.as_str()) {
                    return false;
                }
                query.is_empty()
                    || s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
            })
            .map(|s| format!("{}: {}", s.name, s.description))
            .collect();

        if matches.is_empty() {
            Ok("no skills found".to_owned())
        } else {
            Ok(matches.join("\n"))
        }
    }
}

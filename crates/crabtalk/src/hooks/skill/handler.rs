//! Skill tool — as a Hook implementation.
//!
//! Provides skill loading/discovery and slash-skill preprocessing.

use crate::daemon::hook::AgentScope;
use parking_lot::RwLock;
use runtime::Hook;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool, storage::Skill};

/// Load a skill by name. Returns its instructions on exact match, or lists matching skills otherwise.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct SkillTool {
    /// Skill name to load. If no exact match, returns fuzzy matches.
    /// Leave empty to list all available skills.
    pub name: String,
}

/// Skill subsystem: tool dispatch + slash-skill preprocessing.
///
/// Owns a snapshot of the discovered skills (loaded once at construction
/// from disk) and a scopes reference for enforcing per-agent skill
/// whitelists. Skill changes on disk after construction don't show up
/// until the daemon restarts — same trade-off the rest of the storage
/// layer makes.
pub struct SkillHook {
    skills: Vec<Skill>,
    scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
}

impl SkillHook {
    pub fn new(skills: Vec<Skill>, scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>) -> Self {
        Self { skills, scopes }
    }

    fn find_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }
}

impl Hook for SkillHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![SkillTool::as_tool()]
    }

    fn system_prompt(&self) -> Option<String> {
        build_skill_prompt(&self.skills)
    }

    fn scoped_tools(&self, config: &wcore::AgentConfig) -> (Vec<String>, Option<String>) {
        if config.skills.is_empty() {
            return (vec![], None);
        }
        let tools = self
            .schema()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        let line = format!("skills: {}", config.skills.join(", "));
        (tools, Some(line))
    }

    fn preprocess(&self, agent: &str, content: &str) -> Option<String> {
        let trimmed = content.trim_start();
        let rest = trimmed.strip_prefix('/')?;

        let end = rest
            .find(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-')
            .unwrap_or(rest.len());
        let name = &rest[..end];
        let remainder = &rest[end..];

        if name.is_empty() || name.contains("..") {
            return None;
        }

        // Enforce skill scope.
        {
            let scopes = self.scopes.read();
            if let Some(scope) = scopes.get(agent)
                && !scope.skills.is_empty()
                && !scope.skills.iter().any(|s| s == name)
            {
                return None;
            }
        }

        let skill = self.find_skill(name)?;
        let body = remainder.trim_start();
        let block = format!("<skill name=\"{name}\">\n{}\n</skill>", skill.body);
        if body.is_empty() {
            Some(block)
        } else {
            Some(format!("{body}\n\n{block}"))
        }
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if name != "skill" {
            return None;
        }
        Some(Box::pin(async move {
            let input: SkillTool =
                serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
            let name = &input.name;

            // Enforce skill scope.
            {
                let scopes = self.scopes.read();
                if let Some(scope) = scopes.get(&call.agent)
                    && !scope.skills.is_empty()
                    && !scope.skills.iter().any(|s| s == name)
                {
                    return Err(format!("skill not available: {name}"));
                }
            }

            if name.contains("..") || name.contains('/') || name.contains('\\') {
                return Err(format!("invalid skill name: {name}"));
            }

            if !name.is_empty()
                && let Some(skill) = self.find_skill(name)
            {
                return Ok(skill.body.clone());
            }

            let query = name.to_lowercase();
            let allowed: Vec<String> = self
                .scopes
                .read()
                .get(&call.agent)
                .map(|s| s.skills.clone())
                .unwrap_or_default();

            let matches: Vec<String> = self
                .skills
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
        }))
    }
}

fn build_skill_prompt(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let lines: Vec<String> = skills
        .iter()
        .map(|s| {
            if s.description.is_empty() {
                format!("- {}", s.name)
            } else {
                format!("- {}: {}", s.name, s.description)
            }
        })
        .collect();
    Some(format!(
        "\n\n<resources>\nSkills:\n\
         When a <skill> tag appears in a message, it has been pre-loaded by the system. \
         Follow its instructions directly — do not announce or re-load it.\n\
         Use the skill tool to discover available skills or load one by name.\n{}\n</resources>",
        lines.join("\n")
    ))
}

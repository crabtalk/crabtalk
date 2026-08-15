//! Skill discovery — scan roots for `SKILL.md` directories.
//!
//! Shared by every backend: a skill is a markdown file on disk, not
//! persisted state, so the medium a backend uses for sessions and agents
//! has no bearing on where its skills come from.

use anyhow::Result;
use std::{collections::HashSet, path::PathBuf};
use tokio::fs;
use wcore::storage::Skill;

/// Every skill across `roots`, first root winning on name collisions.
pub(crate) async fn list(roots: &[PathBuf]) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut entries = match fs::read_dir(root).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if !n.starts_with('.') => n.to_owned(),
                _ => continue,
            };
            if seen.contains(&name) {
                continue;
            }
            let skill_path = path.join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let content = match fs::read_to_string(&skill_path).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to read {}: {e}", skill_path.display());
                    continue;
                }
            };
            match hooks::skill::loader::parse_skill_md(&content) {
                Ok(skill) => {
                    seen.insert(name);
                    skills.push(skill);
                }
                Err(e) => tracing::warn!("failed to parse {}: {e}", skill_path.display()),
            }
        }
    }
    Ok(skills)
}

/// One skill by name, searching `roots` in order.
pub(crate) async fn load(roots: &[PathBuf], name: &str) -> Result<Option<Skill>> {
    for root in roots {
        let skill_path = root.join(name).join("SKILL.md");
        if !skill_path.exists() {
            continue;
        }
        let content = fs::read_to_string(&skill_path).await?;
        return Ok(Some(hooks::skill::loader::parse_skill_md(&content)?));
    }
    Ok(None)
}

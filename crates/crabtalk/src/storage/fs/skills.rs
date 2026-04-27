//! Skill discovery — scan `skill_roots` for `SKILL.md` directories.

use super::FsStorage;
use anyhow::Result;
use std::collections::HashSet;
use tokio::fs;
use wcore::storage::Skill;

pub(super) async fn list_skills(storage: &FsStorage) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();
    let mut seen = HashSet::new();
    for root in &storage.skill_roots {
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
            match crate::hooks::skill::loader::parse_skill_md(&content) {
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

pub(super) async fn load_skill(storage: &FsStorage, name: &str) -> Result<Option<Skill>> {
    for root in &storage.skill_roots {
        let skill_path = root.join(name).join("SKILL.md");
        if !skill_path.exists() {
            continue;
        }
        let content = fs::read_to_string(&skill_path).await?;
        let skill = crate::hooks::skill::loader::parse_skill_md(&content)?;
        return Ok(Some(skill));
    }
    Ok(None)
}

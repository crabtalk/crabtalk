//! Filesystem-backed [`SkillRepo`] implementation.
//!
//! Scans multiple roots (user, project, packages) for `<name>/SKILL.md`
//! files. First-root-wins on name conflicts. Hidden directories are
//! skipped.

use anyhow::Result;
use std::{fs, path::PathBuf};
use wcore::repos::{Skill, SkillRepo};

pub struct FsSkillRepo {
    /// Ordered list of roots to scan (local first, then packages).
    roots: Vec<PathBuf>,
    /// Skill names to exclude.
    disabled: Vec<String>,
}

impl FsSkillRepo {
    pub fn new(roots: Vec<PathBuf>, disabled: Vec<String>) -> Self {
        Self { roots, disabled }
    }
}

impl SkillRepo for FsSkillRepo {
    fn list(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            let entries = match fs::read_dir(root) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if !n.starts_with('.') => n.to_owned(),
                    _ => continue,
                };
                if seen.contains(&name) || self.disabled.contains(&name) {
                    continue;
                }
                let skill_path = path.join("SKILL.md");
                if !skill_path.exists() {
                    continue;
                }
                let content = match fs::read_to_string(&skill_path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("failed to read {}: {e}", skill_path.display());
                        continue;
                    }
                };
                match runtime::skill::loader::parse_skill_md(&content) {
                    Ok(skill) => {
                        seen.insert(name);
                        skills.push(skill);
                    }
                    Err(e) => {
                        tracing::warn!("failed to parse {}: {e}", skill_path.display());
                    }
                }
            }
        }
        Ok(skills)
    }

    fn load(&self, name: &str) -> Result<Option<Skill>> {
        for root in &self.roots {
            let skill_path = root.join(name).join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let content = fs::read_to_string(&skill_path)?;
            let skill = runtime::skill::loader::parse_skill_md(&content)?;
            return Ok(Some(skill));
        }
        Ok(None)
    }
}

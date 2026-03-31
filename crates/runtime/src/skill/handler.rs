//! Crabtalk skill handler — initial load from disk.

use crate::skill::{SkillRegistry, loader};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::Mutex;

/// Skill registry owner.
pub struct SkillHandler {
    /// The skill registry (Mutex for interior-mutability from `dispatch_load_skill`).
    pub registry: Mutex<SkillRegistry>,
    /// Skill directories to search (local first, then packages).
    pub skill_dirs: Vec<PathBuf>,
}

impl Default for SkillHandler {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SkillRegistry::new()),
            skill_dirs: Vec::new(),
        }
    }
}

impl SkillHandler {
    /// Load skills from multiple directories. Tolerates missing directories
    /// by skipping them. Skills whose names appear in `disabled` are excluded.
    pub fn load(skill_dirs: Vec<PathBuf>, disabled: &[String]) -> Result<Self> {
        let mut registry = SkillRegistry::new();
        for dir in &skill_dirs {
            if !dir.exists() {
                continue;
            }
            match loader::load_skills_dir(dir) {
                Ok(r) => {
                    let count = r.skills.len();
                    for skill in &r.skills {
                        if disabled.contains(&skill.name) {
                            tracing::info!("skill '{}' disabled, skipping", skill.name);
                        } else if registry.contains(&skill.name) {
                            tracing::warn!(
                                "skill '{}' from {} conflicts with already-loaded skill, skipping",
                                skill.name,
                                dir.display()
                            );
                        } else {
                            registry.add(skill.clone());
                        }
                    }
                    tracing::info!("loaded {count} skill(s) from {}", dir.display());
                }
                Err(e) => {
                    tracing::warn!("could not load skills from {}: {e}", dir.display());
                }
            }
        }
        tracing::info!("total {} skill(s) loaded", registry.skills.len());
        Ok(Self {
            registry: Mutex::new(registry),
            skill_dirs,
        })
    }
}

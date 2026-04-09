//! Crabtalk skill handler — initial load through [`Storage`] backends.

use crate::skill::{SkillRegistry, loader};
use anyhow::Result;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use wcore::Storage;

/// A single root the skill handler reads from. `label` is the display path
/// (used in log messages and the dispatch tool's `Skill directory:`
/// hint); `storage` is a [`Storage`] rooted at that path, through which
/// actual reads happen.
pub struct SkillRoot {
    pub label: PathBuf,
    pub storage: Arc<dyn Storage>,
}

/// Skill registry owner.
pub struct SkillHandler {
    /// The skill registry (Mutex for interior-mutability from `dispatch_load_skill`).
    pub registry: Mutex<SkillRegistry>,
    /// Ordered list of skill roots to search (local first, then packages).
    pub roots: Vec<SkillRoot>,
}

impl Default for SkillHandler {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SkillRegistry::new()),
            roots: Vec::new(),
        }
    }
}

impl SkillHandler {
    /// Load skills from the given roots. Tolerates missing/empty roots.
    /// Skills whose names appear in `disabled` are excluded; duplicates
    /// from later roots are skipped with a warning (first root wins).
    pub fn load(roots: Vec<SkillRoot>, disabled: &[String]) -> Result<Self> {
        let mut registry = SkillRegistry::new();
        for root in &roots {
            match loader::load_skills_from_storage(root.storage.as_ref()) {
                Ok(r) => {
                    let count = r.skills.len();
                    for skill in &r.skills {
                        if disabled.contains(&skill.name) {
                            tracing::info!("skill '{}' disabled, skipping", skill.name);
                        } else if registry.contains(&skill.name) {
                            tracing::warn!(
                                "skill '{}' from {} conflicts with already-loaded skill, skipping",
                                skill.name,
                                root.label.display()
                            );
                        } else {
                            registry.add(skill.clone());
                        }
                    }
                    tracing::info!("loaded {count} skill(s) from {}", root.label.display());
                }
                Err(e) => {
                    tracing::warn!("could not load skills from {}: {e}", root.label.display());
                }
            }
        }
        tracing::info!("total {} skill(s) loaded", registry.skills.len());
        Ok(Self {
            registry: Mutex::new(registry),
            roots,
        })
    }
}

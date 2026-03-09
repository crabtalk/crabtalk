//! Walrus skill handler — initial load from disk.

use crate::hook::skill::{SkillRegistry, loader};
use anyhow::Result;
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use tokio::sync::Mutex;

#[derive(Deserialize, JsonSchema)]
pub(crate) struct SearchSkillInput {
    /// Keyword to match skill names and descriptions. Leave empty to list all.
    pub query: String,
}

#[derive(Deserialize, JsonSchema)]
pub(crate) struct LoadSkillInput {
    /// Skill name
    pub name: String,
}

/// Skill registry owner.
///
/// Implements [`Hook`] — `on_build_agent` enriches the system prompt with
/// matching skills based on agent tags. Tools and dispatch are no-ops
/// (skills inject behavior via prompt, not via tools).
pub struct SkillHandler {
    /// The skill registry (Mutex for interior-mutability from `dispatch_load_skill`).
    pub registry: Mutex<SkillRegistry>,
    /// Base directory from which skills are loaded.
    pub skills_dir: PathBuf,
}

impl Default for SkillHandler {
    fn default() -> Self {
        Self {
            registry: Mutex::new(SkillRegistry::new()),
            skills_dir: PathBuf::new(),
        }
    }
}

impl SkillHandler {
    /// Load skills from the given directory. Tolerates a missing directory
    /// by creating an empty registry.
    pub fn load(skills_dir: PathBuf) -> Result<Self> {
        let registry = if skills_dir.exists() {
            match loader::load_skills_dir(&skills_dir) {
                Ok(r) => {
                    tracing::info!("loaded {} skill(s)", r.len());
                    r
                }
                Err(e) => {
                    tracing::warn!("could not load skills from {}: {e}", skills_dir.display());
                    SkillRegistry::new()
                }
            }
        } else {
            SkillRegistry::new()
        };
        Ok(Self {
            registry: Mutex::new(registry),
            skills_dir,
        })
    }
}

//! Skill discovery — delegates to the SKILL.md standard.

use crate::fs::FsStorage;
use anyhow::Result;
use skill::{Skill, discover};

impl FsStorage {
    pub(super) async fn list_skills(&self) -> Result<Vec<Skill>> {
        discover::list(&self.skill_roots).await
    }

    pub(super) async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        discover::load(&self.skill_roots, name).await
    }
}

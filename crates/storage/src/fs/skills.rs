//! Skill discovery — delegates to the shared filesystem scan.

use crate::{fs::FsStorage, skills};
use anyhow::Result;
use wcore::storage::Skill;

impl FsStorage {
    pub(super) async fn list_skills(&self) -> Result<Vec<Skill>> {
        skills::list(&self.skill_roots).await
    }

    pub(super) async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        skills::load(&self.skill_roots, name).await
    }
}

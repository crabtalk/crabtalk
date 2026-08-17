//! `impl Skills for Store`.

use crate::{
    Skill, SkillSummary,
    interface::Skills,
    kv::{Column, KVStorage},
    sql::SqlIndex,
    store::Store,
};
use anyhow::Result;
use std::str::FromStr;

impl<K: KVStorage, Q: SqlIndex> Skills for Store<K, Q> {
    async fn list_skills(&self, limit: usize, offset: usize) -> Result<Vec<SkillSummary>> {
        self.index.skill_summaries(limit, offset).await
    }

    async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        let key = self.tenant.key(&["skill", name]);
        let Some(bytes) = self.kv.get(Column::Skill, &key).await? else {
            return Ok(None);
        };
        Ok(Some(Skill::from_str(&String::from_utf8(bytes)?)?))
    }

    async fn put_skill(&self, markdown: &str) -> Result<SkillSummary> {
        let skill = Skill::from_str(markdown)?;
        let key = self.tenant.key(&["skill", &skill.name]);
        self.kv
            .put(Column::Skill, &key, markdown.as_bytes())
            .await?;
        let summary = SkillSummary::from(&skill);
        self.index.index_skill(&summary).await?;
        Ok(summary)
    }

    async fn remove_skill(&self, name: &str) -> Result<bool> {
        let indexed = self.index.unindex_skill(name).await?;
        let key = self.tenant.key(&["skill", name]);
        let stored = self.kv.delete(Column::Skill, &key).await?;
        Ok(indexed || stored)
    }
}

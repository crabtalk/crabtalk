//! `impl Skills for Store`.

use crate::{
    Skill, SkillSummary,
    interface::Skills,
    kv::{Column, KVStorage},
    store::Store,
    text::TextSearch,
};
use anyhow::Result;
use std::str::FromStr;

impl<K: KVStorage, T: TextSearch> Skills for Store<K, T> {
    /// Reads summaries and never bodies — they are separate keys, so
    /// that is a property of the layout rather than a rule to remember.
    async fn list_skills(&self, limit: usize, offset: usize) -> Result<Vec<SkillSummary>> {
        let keys = self
            .kv
            .scan_keys(Column::Skill, &self.skill_meta_prefix())
            .await?;
        let mut out = Vec::new();
        for key in keys.iter().skip(offset).take(limit) {
            if let Some(summary) = self.get_json(Column::Skill, key).await? {
                out.push(summary);
            }
        }
        Ok(out)
    }

    async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        let Some(bytes) = self
            .kv
            .get(Column::Skill, &self.skill_body_key(name))
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(Skill::from_str(&String::from_utf8(bytes)?)?))
    }

    async fn put_skill(&self, markdown: &str) -> Result<SkillSummary> {
        let skill = Skill::from_str(markdown)?;
        let summary = SkillSummary::from(&skill);
        self.kv
            .put(
                Column::Skill,
                &self.skill_body_key(&skill.name),
                markdown.as_bytes(),
            )
            .await?;
        self.put_json(Column::Skill, &self.skill_meta_key(&skill.name), &summary)
            .await?;
        Ok(summary)
    }

    async fn remove_skill(&self, name: &str) -> Result<bool> {
        let had_meta = self
            .kv
            .delete(Column::Skill, &self.skill_meta_key(name))
            .await?;
        let had_body = self
            .kv
            .delete(Column::Skill, &self.skill_body_key(name))
            .await?;
        Ok(had_meta || had_body)
    }
}

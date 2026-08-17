//! `impl Agents for Store`.

use crate::{
    AgentConfig, AgentId,
    interface::Agents,
    kv::{Column, KVStorage},
    store::Store,
    text::TextSearch,
    utils::validate_table_name,
};
use anyhow::Result;
use std::str::FromStr;

impl<K: KVStorage, T: TextSearch> Agents for Store<K, T> {
    async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        self.get_json(Column::Agent, &self.agent_key(id)).await
    }

    async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let Some(id) = self.agent_id(name).await? else {
            return Ok(None);
        };
        self.load_agent(&id).await
    }

    /// Ids in name order, straight out of the name index — the keys are
    /// already sorted, so ordering costs nothing and no config is read.
    async fn agent_ids(&self) -> Result<Vec<AgentId>> {
        let rows = self
            .kv
            .scan(Column::Agent, &self.agent_name_prefix())
            .await?;
        Ok(rows
            .iter()
            .filter_map(|(_, id)| AgentId::from_str(std::str::from_utf8(id).ok()?).ok())
            .collect())
    }

    async fn upsert_agent(&self, config: &AgentConfig) -> Result<()> {
        validate_table_name("agent", &config.name)?;
        // An update that changes the name would otherwise leave the old
        // one resolving to this id forever.
        if let Some(previous) = self.load_agent(&config.id).await?
            && previous.name != config.name
        {
            self.kv
                .delete(Column::Agent, &self.agent_name_key(&previous.name))
                .await?;
        }
        // Content first: an index entry is recoverable from content,
        // whereas content the index does not know about is only
        // invisible.
        self.put_json(Column::Agent, &self.agent_key(&config.id), config)
            .await?;
        self.put_name_index(&config.name, &config.id).await
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        if let Some(config) = self.load_agent(id).await? {
            self.kv
                .delete(Column::Agent, &self.agent_name_key(&config.name))
                .await?;
        }
        self.kv.delete(Column::Agent, &self.agent_key(id)).await
    }

    async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        validate_table_name("agent", new_name)?;
        let Some(mut config) = self.load_agent(id).await? else {
            return Ok(false);
        };
        let old_name = std::mem::replace(&mut config.name, new_name.to_owned());
        self.put_json(Column::Agent, &self.agent_key(id), &config)
            .await?;
        self.put_name_index(new_name, id).await?;
        self.kv
            .delete(Column::Agent, &self.agent_name_key(&old_name))
            .await?;
        Ok(true)
    }

    async fn default_agent(&self) -> Result<Option<AgentId>> {
        let key = self.default_agent_key();
        let Some(bytes) = self.kv.get(Column::Config, &key).await? else {
            return Ok(None);
        };
        Ok(AgentId::from_str(std::str::from_utf8(&bytes)?).ok())
    }

    async fn set_default_agent(&self, id: &AgentId) -> Result<()> {
        let key = self.default_agent_key();
        self.kv
            .put(Column::Config, &key, id.to_string().as_bytes())
            .await
    }
}

impl<K: KVStorage, T: TextSearch> Store<K, T> {
    fn default_agent_key(&self) -> Vec<u8> {
        self.tenant.key(&["default_agent"])
    }

    async fn agent_id(&self, name: &str) -> Result<Option<AgentId>> {
        let Some(bytes) = self
            .kv
            .get(Column::Agent, &self.agent_name_key(name))
            .await?
        else {
            return Ok(None);
        };
        Ok(AgentId::from_str(std::str::from_utf8(&bytes)?).ok())
    }

    async fn put_name_index(&self, name: &str, id: &AgentId) -> Result<()> {
        self.kv
            .put(
                Column::Agent,
                &self.agent_name_key(name),
                id.to_string().as_bytes(),
            )
            .await
    }
}

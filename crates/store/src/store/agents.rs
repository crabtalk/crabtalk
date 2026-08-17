//! `impl Agents for Store`.

use crate::{
    AgentConfig, AgentId,
    interface::Agents,
    kv::{Column, KVStorage},
    sql::SqlIndex,
    store::Store,
    utils::validate_table_name,
};
use anyhow::Result;
use std::str::FromStr;

impl<K: KVStorage, Q: SqlIndex> Agents for Store<K, Q> {
    async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        self.get_json(Column::Agent, &self.agent_key(id)).await
    }

    async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let Some(id) = self.index.agent_id_by_name(name).await? else {
            return Ok(None);
        };
        self.load_agent(&id).await
    }

    async fn agent_ids(&self) -> Result<Vec<AgentId>> {
        self.index.agent_ids().await
    }

    async fn upsert_agent(&self, config: &AgentConfig) -> Result<()> {
        validate_table_name("agent", &config.name)?;
        // Content first: an index row is recoverable from KV, whereas a
        // key the index does not know about is merely invisible.
        self.put_json(Column::Agent, &self.agent_key(&config.id), config)
            .await?;
        self.index.index_agent(&config.id, &config.name).await
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let indexed = self.index.unindex_agent(id).await?;
        let stored = self.kv.delete(Column::Agent, &self.agent_key(id)).await?;
        Ok(indexed || stored)
    }

    async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        validate_table_name("agent", new_name)?;
        let Some(mut config) = self.load_agent(id).await? else {
            return Ok(false);
        };
        config.name = new_name.to_owned();
        self.put_json(Column::Agent, &self.agent_key(id), &config)
            .await?;
        self.index.rename_agent(id, new_name).await
    }

    async fn default_agent(&self) -> Result<Option<AgentId>> {
        let key = self.tenant.key(&["default_agent"]);
        let Some(bytes) = self.kv.get(Column::Config, &key).await? else {
            return Ok(None);
        };
        Ok(AgentId::from_str(&String::from_utf8(bytes)?).ok())
    }

    async fn set_default_agent(&self, id: &AgentId) -> Result<()> {
        let key = self.tenant.key(&["default_agent"]);
        self.kv
            .put(Column::Config, &key, id.to_string().as_bytes())
            .await
    }
}

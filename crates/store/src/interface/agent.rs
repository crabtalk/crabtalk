//! Persisted agents.

use crate::{
    AgentConfig, AgentId,
    kv::{Column, KVStorage},
};
use anyhow::Result;
use std::{future::Future, str::FromStr};

/// Persisted agents.
///
/// An agent is addressed by id everywhere but the one place a person
/// types its name, so the name is a lookup rather than an identity —
/// which is what lets a rename leave every session keyed to it alone.
pub trait Agents: Send + Sync + 'static {
    fn load_agent(&self, id: &AgentId) -> impl Future<Output = Result<Option<AgentConfig>>> + Send;

    fn load_agent_by_name(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<AgentConfig>>> + Send;

    /// Ids in name order.
    fn agent_ids(&self) -> impl Future<Output = Result<Vec<AgentId>>> + Send;

    fn upsert_agent(&self, config: &AgentConfig) -> impl Future<Output = Result<()>> + Send;

    fn delete_agent(&self, id: &AgentId) -> impl Future<Output = Result<bool>> + Send;

    /// Rename an agent. The id stays stable, so nothing keyed to it moves.
    fn rename_agent(
        &self,
        id: &AgentId,
        new_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// The agent a surface talks to when it names none.
    ///
    /// Store state, not configuration: the daemon writes it, so it does
    /// not belong in a file a person hand-edits. `None` before scaffold,
    /// or if it points at an agent that has since been deleted.
    fn default_agent(&self) -> impl Future<Output = Result<Option<AgentId>>> + Send;

    fn set_default_agent(&self, id: &AgentId) -> impl Future<Output = Result<()>> + Send;
}

/// Content lives at `agent/{id}`; `idx/agent/{name}` is the only way to
/// reach one by the label a person types.
impl<T: KVStorage> Agents for T {
    async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        self.get_json(Column::Agent, &self.agent_key(id)).await
    }

    async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let Some(id) = self.agent_id(name).await? else {
            return Ok(None);
        };
        self.load_agent(&id).await
    }

    /// Straight out of the name index — the keys are already sorted, so
    /// ordering costs nothing and no config is read.
    async fn agent_ids(&self) -> Result<Vec<AgentId>> {
        let rows = self
            .scan(Column::Agent, &self.prefix(&["idx", "agent"]))
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
            self.delete(Column::Agent, &self.agent_name_key(&previous.name))
                .await?;
        }
        // Content first: an index entry is recoverable from content,
        // whereas content the index does not know about is only
        // invisible.
        self.put_json(Column::Agent, &self.agent_key(&config.id), config)
            .await?;
        self.put_agent_name(&config.name, &config.id).await
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        if let Some(config) = self.load_agent(id).await? {
            self.delete(Column::Agent, &self.agent_name_key(&config.name))
                .await?;
        }
        self.delete(Column::Agent, &self.agent_key(id)).await
    }

    async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        validate_table_name("agent", new_name)?;
        let Some(mut config) = self.load_agent(id).await? else {
            return Ok(false);
        };
        let old_name = std::mem::replace(&mut config.name, new_name.to_owned());
        self.put_json(Column::Agent, &self.agent_key(id), &config)
            .await?;
        self.put_agent_name(new_name, id).await?;
        self.delete(Column::Agent, &self.agent_name_key(&old_name))
            .await?;
        Ok(true)
    }

    async fn default_agent(&self) -> Result<Option<AgentId>> {
        let Some(bytes) = self
            .get(Column::Config, &self.key(&["default_agent"]))
            .await?
        else {
            return Ok(None);
        };
        Ok(AgentId::from_str(std::str::from_utf8(&bytes)?).ok())
    }

    async fn set_default_agent(&self, id: &AgentId) -> Result<()> {
        self.put(
            Column::Config,
            &self.key(&["default_agent"]),
            id.to_string().as_bytes(),
        )
        .await
    }
}

/// The keyspace agents are filed under. Private: a store that holds them
/// in a table of its own has none of these.
trait AgentKv: KVStorage {
    fn agent_key(&self, id: &AgentId) -> Vec<u8> {
        self.key(&["agent", &id.to_string()])
    }

    fn agent_name_key(&self, name: &str) -> Vec<u8> {
        self.key(&["idx", "agent", name])
    }

    fn agent_id(&self, name: &str) -> impl Future<Output = Result<Option<AgentId>>> + Send {
        async move {
            let Some(bytes) = self.get(Column::Agent, &self.agent_name_key(name)).await? else {
                return Ok(None);
            };
            Ok(AgentId::from_str(std::str::from_utf8(&bytes)?).ok())
        }
    }

    fn put_agent_name(&self, name: &str, id: &AgentId) -> impl Future<Output = Result<()>> + Send {
        async move {
            self.put(
                Column::Agent,
                &self.agent_name_key(name),
                id.to_string().as_bytes(),
            )
            .await
        }
    }
}

impl<T: KVStorage + ?Sized> AgentKv for T {}

/// Reject names that won't survive serialization as a TOML table key.
///
/// Agent names are the one identifier a person types, so they are the
/// one that can arrive malformed.
pub fn validate_table_name(kind: &str, name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("{kind}: name must not be empty");
    }
    if name
        .chars()
        .any(|c| matches!(c, '.' | '[' | ']' | '"') || c.is_control())
    {
        anyhow::bail!(
            "{kind}: name '{name}' must not contain '.', '[', ']', '\"', or control chars"
        );
    }
    Ok(())
}

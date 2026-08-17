//! Persisted agents.

use crate::{
    AgentConfig, AgentId,
    kv::{Column, KVStorage},
};
use anyhow::Result;
use std::{future::Future, str::FromStr};

/// Persisted agents.
///
/// Content lives at `agent/{id}`; `idx/agent/{name}` is the only way to
/// reach one by the label a person types, and everything else addresses
/// it by id.
pub trait Agents: KVStorage {
    fn load_agent(&self, id: &AgentId) -> impl Future<Output = Result<Option<AgentConfig>>> + Send {
        async move {
            self.get_json(Column::Agent, &self.key(&["agent", &id.to_string()]))
                .await
        }
    }

    fn load_agent_by_name(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<AgentConfig>>> + Send {
        async move {
            let Some(id) = self.agent_id(name).await? else {
                return Ok(None);
            };
            self.load_agent(&id).await
        }
    }

    /// Ids in name order, straight out of the name index — the keys are
    /// already sorted, so ordering costs nothing and no config is read.
    fn agent_ids(&self) -> impl Future<Output = Result<Vec<AgentId>>> + Send {
        async move {
            let rows = self
                .scan(Column::Agent, &self.prefix(&["idx", "agent"]))
                .await?;
            Ok(rows
                .iter()
                .filter_map(|(_, id)| AgentId::from_str(std::str::from_utf8(id).ok()?).ok())
                .collect())
        }
    }

    fn upsert_agent(&self, config: &AgentConfig) -> impl Future<Output = Result<()>> + Send {
        async move {
            validate_table_name("agent", &config.name)?;
            // An update that changes the name would otherwise leave the
            // old one resolving to this id forever.
            if let Some(previous) = self.load_agent(&config.id).await?
                && previous.name != config.name
            {
                self.delete(Column::Agent, &self.agent_name_key(&previous.name))
                    .await?;
            }
            // Content first: an index entry is recoverable from content,
            // whereas content the index does not know about is only
            // invisible.
            self.put_json(
                Column::Agent,
                &self.key(&["agent", &config.id.to_string()]),
                config,
            )
            .await?;
            self.put_agent_name(&config.name, &config.id).await
        }
    }

    fn delete_agent(&self, id: &AgentId) -> impl Future<Output = Result<bool>> + Send {
        async move {
            if let Some(config) = self.load_agent(id).await? {
                self.delete(Column::Agent, &self.agent_name_key(&config.name))
                    .await?;
            }
            self.delete(Column::Agent, &self.key(&["agent", &id.to_string()]))
                .await
        }
    }

    /// Rename an agent. The id stays stable, so nothing keyed to it moves.
    fn rename_agent(
        &self,
        id: &AgentId,
        new_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send {
        async move {
            validate_table_name("agent", new_name)?;
            let Some(mut config) = self.load_agent(id).await? else {
                return Ok(false);
            };
            let old_name = std::mem::replace(&mut config.name, new_name.to_owned());
            self.put_json(
                Column::Agent,
                &self.key(&["agent", &id.to_string()]),
                &config,
            )
            .await?;
            self.put_agent_name(new_name, id).await?;
            self.delete(Column::Agent, &self.agent_name_key(&old_name))
                .await?;
            Ok(true)
        }
    }

    /// The agent a surface talks to when it names none.
    ///
    /// Store state, not configuration: the daemon writes it, so it does
    /// not belong in a file a person hand-edits. `None` before scaffold,
    /// or if it points at an agent that has since been deleted.
    fn default_agent(&self) -> impl Future<Output = Result<Option<AgentId>>> + Send {
        async move {
            let Some(bytes) = self
                .get(Column::Config, &self.key(&["default_agent"]))
                .await?
            else {
                return Ok(None);
            };
            Ok(AgentId::from_str(std::str::from_utf8(&bytes)?).ok())
        }
    }

    fn set_default_agent(&self, id: &AgentId) -> impl Future<Output = Result<()>> + Send {
        async move {
            self.put(
                Column::Config,
                &self.key(&["default_agent"]),
                id.to_string().as_bytes(),
            )
            .await
        }
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

impl<T: KVStorage> Agents for T {}

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

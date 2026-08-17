//! The backed implementation of every interface.
//!
//! One struct over the two primitives. Content goes to KV; so do the
//! secondary indexes, because an index is just more keys. Only ranked
//! full-text goes to [`TextSearch`](crate::TextSearch) — it is the one
//! lookup keys cannot answer.
//!
//! ```text
//! Column::Agent    agent/{id}                              AgentConfig
//!                  idx/agent/{name}                        id
//! Column::Session  session/{handle}/meta                   SessionMeta
//!                  session/{handle}/archive                memory entry name
//!                  session/{handle}/msg/{idx:012}          HistoryEntry
//!                  session/{handle}/evt/{idx:012}          EventLine
//!                  idx/sess/{agent}/{by}/{created_at}/{h}  handle
//! Column::Memory   memory/{name}                           MemoryEntry
//! Column::Skill    skill/meta/{name}                       SkillSummary
//!                  skill/body/{name}                       SKILL.md
//! Column::Harness  image/{digest}                          ELF
//!                  name/{name}                             digest
//! Column::Config   default_agent                           id
//! ```
//!
//! Two shapes, each chosen by its dominant access. A session's keys nest
//! under its handle so deleting one is a single prefix sweep. A skill's
//! metadata and body are split so a listing reads names without touching
//! markdown — the property is structural rather than a rule the backend
//! has to remember.
//!
//! Writes are ordered content-first, index-second: a crash orphans
//! content nothing can reach, rather than leaving an index entry
//! pointing at nothing. Every index is rebuildable by scanning content.
//!
//! One trait impl per sibling file. What lives here is the struct they
//! share and the keys they address content by — a key format is the one
//! thing every impl has to agree on.

use crate::{
    AgentId,
    kv::{Column, KVStorage, Tenant},
    session::{SessionHandle, SessionMeta},
    text::TextSearch,
};
use anyhow::Result;

mod agents;
mod harnesses;
mod memory;
mod sessions;
mod skills;

/// KV content and the text index derived from it.
pub struct Store<K, T> {
    pub kv: K,
    pub text: T,
    tenant: Tenant,
}

impl<K: KVStorage, T: TextSearch> Store<K, T> {
    pub fn new(kv: K, text: T) -> Self {
        Self {
            kv,
            text,
            tenant: Tenant::default(),
        }
    }

    /// Open a store for one tenant. Local installs use [`Store::new`];
    /// this is here so the key format never has to change for a backend
    /// that serves more than one.
    pub fn with_tenant(kv: K, text: T, tenant: Tenant) -> Self {
        Self { kv, text, tenant }
    }

    async fn get_json<V: serde::de::DeserializeOwned>(
        &self,
        col: Column,
        key: &[u8],
    ) -> Result<Option<V>> {
        let Some(bytes) = self.kv.get(col, key).await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    async fn put_json<V: serde::Serialize>(
        &self,
        col: Column,
        key: &[u8],
        value: &V,
    ) -> Result<()> {
        self.kv.put(col, key, &serde_json::to_vec(value)?).await
    }

    // ── Agents ─────────────────────────────────────────────────────

    fn agent_key(&self, id: &AgentId) -> Vec<u8> {
        self.tenant.key(&["agent", &id.to_string()])
    }

    /// Name → id. The only way to reach an agent by the label a person
    /// types; everything else addresses it by id.
    fn agent_name_key(&self, name: &str) -> Vec<u8> {
        self.tenant.key(&["idx", "agent", name])
    }

    fn agent_name_prefix(&self) -> Vec<u8> {
        self.tenant.prefix(&["idx", "agent"])
    }

    // ── Sessions ───────────────────────────────────────────────────

    fn meta_key(&self, handle: &SessionHandle) -> Vec<u8> {
        self.tenant.key(&["session", handle.as_str(), "meta"])
    }

    fn archive_key(&self, handle: &SessionHandle) -> Vec<u8> {
        self.tenant.key(&["session", handle.as_str(), "archive"])
    }

    fn message_key(&self, handle: &SessionHandle, idx: usize) -> Vec<u8> {
        // Zero-padded so a prefix scan returns messages in order: keys
        // sort as bytes, and "10" sorts before "2".
        self.tenant
            .key(&["session", handle.as_str(), "msg", &format!("{idx:012}")])
    }

    fn message_prefix(&self, handle: &SessionHandle) -> Vec<u8> {
        self.tenant.prefix(&["session", handle.as_str(), "msg"])
    }

    fn event_key(&self, handle: &SessionHandle, idx: usize) -> Vec<u8> {
        self.tenant
            .key(&["session", handle.as_str(), "evt", &format!("{idx:012}")])
    }

    fn session_prefix(&self, handle: &SessionHandle) -> Vec<u8> {
        self.tenant.prefix(&["session", handle.as_str()])
    }

    /// `(agent, created_by, created_at)` → handle.
    ///
    /// Ordered by construction: `created_at` is RFC3339, which sorts
    /// lexicographically, so the newest session for an identity is the
    /// last key under its prefix and needs no query to find.
    fn session_index_key(&self, meta: &SessionMeta, handle: &SessionHandle) -> Vec<u8> {
        self.tenant.key(&[
            "idx",
            "sess",
            &meta.agent.to_string(),
            &meta.created_by,
            &meta.created_at,
            handle.as_str(),
        ])
    }

    fn session_index_prefix(&self, agent: Option<&AgentId>, created_by: Option<&str>) -> Vec<u8> {
        match (agent, created_by) {
            (Some(a), Some(by)) => self.tenant.prefix(&["idx", "sess", &a.to_string(), by]),
            (Some(a), None) => self.tenant.prefix(&["idx", "sess", &a.to_string()]),
            _ => self.tenant.prefix(&["idx", "sess"]),
        }
    }

    /// The message position a text hit points at, given the key it was
    /// indexed under. `None` for a key that is not a message.
    fn parse_message_key(&self, key: &[u8]) -> Option<(SessionHandle, usize)> {
        let key = std::str::from_utf8(key).ok()?;
        let mut parts = key.rsplit('/');
        let idx = parts.next()?.parse().ok()?;
        if parts.next()? != "msg" {
            return None;
        }
        Some((SessionHandle::new(parts.next()?), idx))
    }

    async fn meta(&self, handle: &SessionHandle) -> Result<Option<SessionMeta>> {
        self.get_json(Column::Session, &self.meta_key(handle)).await
    }

    // ── Memory ─────────────────────────────────────────────────────

    fn memory_key(&self, name: &str) -> Vec<u8> {
        self.tenant.key(&["memory", name])
    }

    fn memory_prefix(&self) -> Vec<u8> {
        self.tenant.prefix(&["memory"])
    }

    // ── Skills ─────────────────────────────────────────────────────

    fn skill_meta_key(&self, name: &str) -> Vec<u8> {
        self.tenant.key(&["skill", "meta", name])
    }

    fn skill_body_key(&self, name: &str) -> Vec<u8> {
        self.tenant.key(&["skill", "body", name])
    }

    fn skill_meta_prefix(&self) -> Vec<u8> {
        self.tenant.prefix(&["skill", "meta"])
    }
}

/// The trailing segment of a slash-separated key.
fn last_segment(key: &[u8]) -> Option<String> {
    std::str::from_utf8(key)
        .ok()?
        .rsplit('/')
        .next()
        .map(str::to_owned)
}

//! The backed implementation of every interface.
//!
//! One struct over the two primitives. Content goes to KV under a
//! tenant-prefixed key; whatever a lookup needs to *find* that key goes
//! to the index. Writes are ordered so a crash cannot leave an index row
//! pointing at content that is not there: content first on the way in,
//! index first on the way out.
//!
//! One trait impl per sibling file. What lives here is the struct they
//! share and the keys they address content by — a key format is the one
//! thing every impl has to agree on.

use crate::{
    AgentId,
    kv::{Column, KVStorage, Tenant},
    session::{SessionHandle, SessionMeta},
    sql::SqlIndex,
};
use anyhow::Result;

/// KV content plus the index derived from it.
pub struct Store<K, Q> {
    pub kv: K,
    pub index: Q,
    tenant: Tenant,
}

impl<K: KVStorage, Q: SqlIndex> Store<K, Q> {
    pub fn new(kv: K, index: Q) -> Self {
        Self {
            kv,
            index,
            tenant: Tenant::default(),
        }
    }

    /// Open a store for one tenant. Local installs use [`Store::new`];
    /// this is here so the key format never has to change for a backend
    /// that serves more than one.
    pub fn with_tenant(kv: K, index: Q, tenant: Tenant) -> Self {
        Self { kv, index, tenant }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        col: Column,
        key: &[u8],
    ) -> Result<Option<T>> {
        let Some(bytes) = self.kv.get(col, key).await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    async fn put_json<T: serde::Serialize>(
        &self,
        col: Column,
        key: &[u8],
        value: &T,
    ) -> Result<()> {
        self.kv.put(col, key, &serde_json::to_vec(value)?).await
    }

    fn agent_key(&self, id: &AgentId) -> Vec<u8> {
        self.tenant.key(&["agent", &id.to_string()])
    }

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

    fn event_key(&self, handle: &SessionHandle, idx: usize) -> Vec<u8> {
        self.tenant
            .key(&["session", handle.as_str(), "evt", &format!("{idx:012}")])
    }

    async fn meta(&self, handle: &SessionHandle) -> Result<Option<SessionMeta>> {
        self.get_json(Column::Session, &self.meta_key(handle)).await
    }
}

mod agents;
mod harnesses;
mod memory;
mod sessions;
mod skills;

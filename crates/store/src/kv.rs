//! The KV primitive — the system of record.
//!
//! Everything addressable by a key you already hold lives here: agent
//! configs, skill bodies, memory entries, session blobs, harness images.
//! [`SqlIndex`](crate::SqlIndex) sits beside it holding only what a
//! lookup needs to *find* one of these — ordering fields, FTS terms, set
//! membership — and never content. That asymmetry is the whole design:
//! the index is derived, so it can be rebuilt by scanning a column, and
//! no write ever needs a transaction spanning the two stores.
//!
//! The surface is deliberately small. A query engine would let a caller
//! ask for anything, including something that crosses a realm; four
//! methods over a prefixed keyspace cannot express that.

use anyhow::Result;
use parking_lot::RwLock;
use std::{collections::BTreeMap, future::Future};

/// A hard partition of the keyspace. A scan in one column never sees
/// another's keys, so a prefix cannot collide across kinds.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Column {
    Agent = 0,
    Session = 1,
    Memory = 2,
    Skill = 3,
    Harness = 4,
    Config = 5,
}

impl Column {
    /// Every column, for backends that must enumerate them at open.
    pub const ALL: [Column; 6] = [
        Column::Agent,
        Column::Session,
        Column::Memory,
        Column::Skill,
        Column::Harness,
        Column::Config,
    ];
}

/// The realm slot every key carries.
///
/// One realm is one store here, so this is always [`Realm::LOCAL`] and
/// buys nothing today. It is in the key format from day one anyway: a
/// multi-realm backend is then a different [`KVStorage`] impl rather
/// than a key migration, and a read outside the realm is not
/// expressible rather than merely forbidden.
pub struct Realm;

impl Realm {
    /// The single realm of a local install, and the default every
    /// backend gets until it says otherwise.
    pub const LOCAL: &'static str = "local";
}

/// The key-value primitive.
///
/// Implement the five required methods and you have a backend: the
/// behavior interfaces — [`Agents`](crate::Agents),
/// [`Sessions`](crate::Sessions) and the rest — are blanket-implemented
/// for anything that satisfies this, so nothing above has to be written
/// or wired a second time.
///
/// The provided methods are the keyspace: which realm a key belongs to,
/// how one is built, and how a JSON value round-trips through it. Only
/// [`realm`](KVStorage::realm) is worth overriding, and only by a
/// backend that serves more than one.
pub trait KVStorage: Send + Sync + 'static {
    fn get(&self, col: Column, key: &[u8]) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    fn put(&self, col: Column, key: &[u8], value: &[u8])
    -> impl Future<Output = Result<()>> + Send;

    /// Remove a key. `true` if it was there.
    fn delete(&self, col: Column, key: &[u8]) -> impl Future<Output = Result<bool>> + Send;

    /// Keys under `prefix`, ascending. Keys only — a value scan over a
    /// column holding harness images or skill bodies is a read of every
    /// byte in it, which is never what an enumeration wants.
    fn scan_keys(
        &self,
        col: Column,
        prefix: &[u8],
    ) -> impl Future<Output = Result<Vec<Vec<u8>>>> + Send;

    /// Keys and values under `prefix`, ascending.
    ///
    /// For secondary indexes, where the value is the primary key being
    /// pointed at and is small by construction. Never reach for this on
    /// a prefix holding content.
    fn scan(
        &self,
        col: Column,
        prefix: &[u8],
    ) -> impl Future<Output = Result<Vec<(Vec<u8>, Vec<u8>)>>> + Send;

    // ── The keyspace ───────────────────────────────────────────────

    /// The realm every key of this store belongs to.
    ///
    /// One store is one realm here, so the default is the only answer.
    /// A backend serving many overrides it, and because the slot is
    /// already in every key, that is the whole change — no migration,
    /// and a read outside the realm stops being expressible.
    fn realm(&self) -> &str {
        Realm::LOCAL
    }

    /// Build a key: `{realm}/{parts joined by '/'}`.
    fn key(&self, parts: &[&str]) -> Vec<u8> {
        let mut key = String::from(self.realm());
        for part in parts {
            key.push('/');
            key.push_str(part);
        }
        key.into_bytes()
    }

    /// The prefix every key under `parts` starts with, for scans. The
    /// trailing separator is what stops `…/msg` from also matching
    /// `…/meta`.
    fn prefix(&self, parts: &[&str]) -> Vec<u8> {
        let mut prefix = self.key(parts);
        prefix.push(b'/');
        prefix
    }

    fn get_json<V: serde::de::DeserializeOwned>(
        &self,
        col: Column,
        key: &[u8],
    ) -> impl Future<Output = Result<Option<V>>> + Send {
        async move {
            let Some(bytes) = self.get(col, key).await? else {
                return Ok(None);
            };
            Ok(Some(serde_json::from_slice(&bytes)?))
        }
    }

    fn put_json<V: serde::Serialize + Sync>(
        &self,
        col: Column,
        key: &[u8],
        value: &V,
    ) -> impl Future<Output = Result<()>> + Send {
        async move { self.put(col, key, &serde_json::to_vec(value)?).await }
    }
}

/// A key qualified by its column.
///
/// Column first so the map's ordering groups by column before key, which
/// is what lets [`MemoryDb::scan_keys`] answer a prefix scan with one
/// range instead of a filter over everything.
type ColumnKey = (u8, Vec<u8>);

/// In-RAM [`KVStorage`]. Independent per instance; nothing is persisted.
#[derive(Debug, Default)]
pub struct MemoryDb {
    entries: RwLock<BTreeMap<ColumnKey, Vec<u8>>>,
}

impl MemoryDb {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KVStorage for MemoryDb {
    async fn get(&self, col: Column, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.entries.read().get(&(col as u8, key.to_vec())).cloned())
    }

    async fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
        self.entries
            .write()
            .insert((col as u8, key.to_vec()), value.to_vec());
        Ok(())
    }

    async fn delete(&self, col: Column, key: &[u8]) -> Result<bool> {
        Ok(self
            .entries
            .write()
            .remove(&(col as u8, key.to_vec()))
            .is_some())
    }

    async fn scan_keys(&self, col: Column, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        let entries = self.entries.read();
        Ok(entries
            .range((col as u8, prefix.to_vec())..)
            .take_while(|((c, key), _)| *c == col as u8 && key.starts_with(prefix))
            .map(|((_, key), _)| key.clone())
            .collect())
    }

    async fn scan(&self, col: Column, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let entries = self.entries.read();
        Ok(entries
            .range((col as u8, prefix.to_vec())..)
            .take_while(|((c, key), _)| *c == col as u8 && key.starts_with(prefix))
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect())
    }
}

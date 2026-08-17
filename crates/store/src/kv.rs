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
//! ask for anything, including something that crosses a tenant; four
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

/// The tenant slot every key carries.
///
/// One tenant is one store here, so this is always [`Tenant::LOCAL`] and
/// buys nothing today. It is in the key format from day one anyway: a
/// multi-tenant backend is then a different [`KVStorage`] impl rather
/// than a key migration, and a read outside the tenant is not
/// expressible rather than merely forbidden.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tenant(String);

impl Default for Tenant {
    fn default() -> Self {
        Self(Self::LOCAL.to_owned())
    }
}

impl Tenant {
    /// The single tenant of a local install.
    pub const LOCAL: &'static str = "local";

    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Build a key: `{tenant}/{parts joined by '/'}`.
    pub fn key(&self, parts: &[&str]) -> Vec<u8> {
        let mut key = String::with_capacity(self.0.len() + 1);
        key.push_str(&self.0);
        for part in parts {
            key.push('/');
            key.push_str(part);
        }
        key.into_bytes()
    }

    /// The prefix every key of this tenant starts with, for scans.
    pub fn prefix(&self, parts: &[&str]) -> Vec<u8> {
        let mut prefix = self.key(parts);
        prefix.push(b'/');
        prefix
    }
}

/// The key-value primitive.
///
/// Implement this and you have a backend. Everything above it —
/// behavior interfaces, the runtime's working sets — is written once
/// against this trait.
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
}

/// A shared handle is a backend. Lets one open database serve as both
/// halves of a [`Store`](crate::Store) — a local install is one file, so
/// its KV and its index are the same connection pool.
impl<T: KVStorage> KVStorage for std::sync::Arc<T> {
    fn get(&self, col: Column, key: &[u8]) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send {
        (**self).get(col, key)
    }

    fn put(
        &self,
        col: Column,
        key: &[u8],
        value: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        (**self).put(col, key, value)
    }

    fn delete(&self, col: Column, key: &[u8]) -> impl Future<Output = Result<bool>> + Send {
        (**self).delete(col, key)
    }

    fn scan_keys(
        &self,
        col: Column,
        prefix: &[u8],
    ) -> impl Future<Output = Result<Vec<Vec<u8>>>> + Send {
        (**self).scan_keys(col, prefix)
    }

    fn scan(
        &self,
        col: Column,
        prefix: &[u8],
    ) -> impl Future<Output = Result<Vec<(Vec<u8>, Vec<u8>)>>> + Send {
        (**self).scan(col, prefix)
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

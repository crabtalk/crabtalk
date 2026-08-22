//! The backend a general install runs.
//!
//! Which store to use is a deployment decision, and storage engines are
//! heavy, so both live here rather than in `crabtalk-store`: a runtime
//! crate has no business linking one. That crate defines the keyspace
//! and everything built on it — agents, sessions, memory, skills,
//! harnesses, and BM25 search across them — against five methods, and
//! this is those five methods over [`CrabDb`].
//!
//! One file per realm, so a realm is a thing you can copy, move, or
//! delete whole.

use anyhow::Result;
use crabdb::CrabDb;
use std::{path::PathBuf, sync::Arc};
use store::kv::{Column, KVStorage};

/// A realm's store.
///
/// Implements [`KVStorage`] and is therefore already an `Agents`, a
/// `Sessions`, a `Memory`, a `Skills`, a `Harnesses` and a `TextSearch`
/// — every one of those is blanket-implemented over the five methods
/// below, so there is nothing here to pair up or wrap.
pub struct Backend {
    db: Arc<CrabDb>,
}

impl Backend {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            db: Arc::new(CrabDb::open(path)?),
        })
    }

    /// Snapshot the key index and fsync. The next open reads instead of
    /// replaying, and everything written so far is durable against power
    /// loss rather than only against a process crash.
    pub fn checkpoint(&self) -> Result<()> {
        self.db.checkpoint()
    }
}

/// Every method hands the work to a blocking thread.
///
/// The store is synchronous — a lookup is a seek and a read — and most
/// calls return in microseconds. Compaction does not: it rewrites the
/// file, and running that on an executor thread would stall every other
/// task in the daemon, including a stream mid-response.
impl KVStorage for Backend {
    async fn get(&self, col: Column, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let (db, key) = (self.db.clone(), key.to_vec());
        tokio::task::spawn_blocking(move || db.get(col as u8, &key)).await?
    }

    async fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
        let (db, key, value) = (self.db.clone(), key.to_vec(), value.to_vec());
        tokio::task::spawn_blocking(move || db.put(col as u8, &key, &value)).await?
    }

    async fn delete(&self, col: Column, key: &[u8]) -> Result<bool> {
        let (db, key) = (self.db.clone(), key.to_vec());
        tokio::task::spawn_blocking(move || db.delete(col as u8, &key)).await?
    }

    async fn scan_keys(&self, col: Column, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        let (db, prefix) = (self.db.clone(), prefix.to_vec());
        tokio::task::spawn_blocking(move || db.scan_keys(col as u8, &prefix)).await?
    }

    async fn scan(&self, col: Column, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let (db, prefix) = (self.db.clone(), prefix.to_vec());
        tokio::task::spawn_blocking(move || db.scan(col as u8, &prefix)).await?
    }
}

/// This backend satisfies everything the runtime asks for.
/// Asserted here because nothing in this workspace instantiates it yet.
const _: fn() = || {
    fn assert_backend<T: store::Backend>() {}
    assert_backend::<Backend>();
};

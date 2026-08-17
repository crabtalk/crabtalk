//! The backend a general install runs.
//!
//! Which store to use is a deployment decision, so it lives in the app
//! rather than in `crabtalk-store`: that crate defines the two
//! primitives and the interfaces built on them, and this one picks
//! sqlite for both halves. Choosing differently — parity-db for content,
//! postgres for the index — is another crate like this one, not a change
//! to anything above it.
//!
//! `kv.rs` implements the content primitive, `index.rs` the queries
//! derived from it, and [`SqliteStore`] pairs them. One database per
//! tenant, so a tenant is a thing you can copy, move, or delete whole.

use anyhow::Result;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::PathBuf, str::FromStr, sync::Arc};
use store::Store;

mod convert;
mod index;
mod kv;
mod schema;
mod search;

/// The shipped local backend.
///
/// One open database serves as both primitives — a local install is one
/// file, so its content and the index derived from it share a pool.
pub type SqliteStore = Store<Arc<SqliteStorage>, Arc<SqliteStorage>>;

/// A tenant's database.
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Open (creating if absent) the database at `path` and apply the
    /// schema.
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            // WAL lets a reader run while a writer holds the lock, which
            // is what makes a single file workable for a live daemon.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().connect_with(options).await?;
        for statement in schema::DDL {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self { pool })
    }

    /// Open the shipped backend at `path`.
    pub async fn store(path: impl Into<PathBuf>) -> Result<SqliteStore> {
        let db = Arc::new(Self::open(path).await?);
        Ok(Store::new(db.clone(), db))
    }

    /// The shipped backend, in RAM. Each call is an independent store.
    pub async fn memory_store() -> Result<SqliteStore> {
        let db = Arc::new(Self::open_in_memory().await?);
        Ok(Store::new(db.clone(), db))
    }

    /// Open an in-memory database. Each call is an independent store.
    pub async fn open_in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // An in-memory database lives as long as its connection, so the
        // pool must hold exactly one and never recycle it.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        for statement in schema::DDL {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self { pool })
    }
}

/// This backend satisfies everything the runtime asks for.
/// Asserted here because nothing in this workspace instantiates it yet.
const _: fn() = || {
    fn assert_backend<T: store::interface::Backend>() {}
    assert_backend::<SqliteStore>();
};

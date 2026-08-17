//! The backend a general install runs.
//!
//! Which store to use is a deployment decision, so it lives in the app
//! rather than in `crabtalk-store`: that crate defines the two
//! primitives and the interfaces built on them, and this one picks
//! sqlite for both halves. Choosing differently — parity-db for content,
//! postgres for the index — is another crate like this one, not a change
//! to anything above it.
//!
//! `kv.rs` implements the content primitive and `text.rs` the ranked
//! search one; [`SqliteStore`] pairs them. Eight methods in total —
//! everything a query used to be written for lives above this, in
//! `Store`. One database per realm, so a realm is a thing you can
//! copy, move, or delete whole.

use anyhow::Result;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::PathBuf, str::FromStr};

mod kv;
mod schema;
mod text;

/// A realm's database.
///
/// Implements both primitives, and is therefore already an `Agents`, a
/// `Sessions`, a `Memory`, a `Skills` and a `Harnesses` — the interfaces
/// carry their own bodies, so there is nothing here to pair up or wrap.
/// A local install is one file, so content and the text index derived
/// from it share a pool.
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

    /// Open in RAM. Each call is an independent store.
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
    fn assert_backend<T: store::Backend>() {}
    assert_backend::<SqliteStorage>();
};

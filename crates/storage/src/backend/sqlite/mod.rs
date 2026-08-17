//! SQLite-backed [`Storage`](crate::Storage).
//!
//! One database per tenant: sessions, agents, and the install config all
//! live in a single file, so a tenant is a thing you can copy, move, or
//! delete whole. Skills stay on the filesystem — they are content, not
//! state, and are read by scanning the skill roots.
//!
//! Each entity's logic lives in inherent methods on [`SqliteStorage`],
//! one file per concern; the `Storage` impl in `storage.rs` forwards to
//! them.
#![cfg(feature = "sqlite")]

use anyhow::Result;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::PathBuf, str::FromStr};

mod agents;
mod config;
mod convert;
mod schema;
mod search;
mod sessions;
mod storage;

/// A tenant's database, plus the roots its skills are read from.
pub struct SqliteStorage {
    pool: SqlitePool,
    /// Ordered skill roots to scan.
    skill_roots: Vec<PathBuf>,
}

impl SqliteStorage {
    /// Open (creating if absent) the database at `path` and apply the
    /// schema. `skill_roots` are scanned for `SKILL.md` directories.
    pub async fn open(path: impl Into<PathBuf>, skill_roots: Vec<PathBuf>) -> Result<Self> {
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
        Ok(Self { pool, skill_roots })
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
        Ok(Self {
            pool,
            skill_roots: Vec::new(),
        })
    }
}

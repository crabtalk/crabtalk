//! [`KVStorage`] over the `kv` table.
//!
//! A local install is one file, so the KV primitive and the index it
//! derives share a database. That is a property of this backend, not of
//! the design: parity-db behind `KVStorage` with postgres behind
//! `SqlIndex` is the same code above, because nothing above depends on
//! the two being one store.

use crate::backend::SqliteStorage;
use anyhow::Result;
use store::kv::{Column, KVStorage};

impl KVStorage for SqliteStorage {
    async fn get(&self, col: Column, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let value: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT value FROM kv WHERE col = ? AND key = ?")
                .bind(col as u8)
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(value)
    }

    async fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO kv (col, key, value) VALUES (?, ?, ?)
             ON CONFLICT(col, key) DO UPDATE SET value = excluded.value",
        )
        .bind(col as u8)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete(&self, col: Column, key: &[u8]) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM kv WHERE col = ? AND key = ?")
            .bind(col as u8)
            .bind(key)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn scan(&self, col: Column, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut upper = prefix.to_vec();
        let rows: Vec<(Vec<u8>, Vec<u8>)> = match next_prefix(&mut upper) {
            true => {
                sqlx::query_as(
                    "SELECT key, value FROM kv
                     WHERE col = ? AND key >= ? AND key < ?
                     ORDER BY key",
                )
                .bind(col as u8)
                .bind(prefix)
                .bind(&upper)
                .fetch_all(&self.pool)
                .await?
            }
            false => {
                sqlx::query_as("SELECT key, value FROM kv WHERE col = ? AND key >= ? ORDER BY key")
                    .bind(col as u8)
                    .bind(prefix)
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        Ok(rows)
    }

    async fn scan_keys(&self, col: Column, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        // `GLOB` would treat `*`, `?` and `[` in a key as syntax. The
        // range is an index seek on the primary key and needs no
        // escaping: every key starting with `prefix` sorts between it and
        // its successor.
        let mut upper = prefix.to_vec();
        let keys: Vec<Vec<u8>> = match next_prefix(&mut upper) {
            true => {
                sqlx::query_scalar(
                    "SELECT key FROM kv
                     WHERE col = ? AND key >= ? AND key < ?
                     ORDER BY key",
                )
                .bind(col as u8)
                .bind(prefix)
                .bind(&upper)
                .fetch_all(&self.pool)
                .await?
            }
            // Every byte was 0xFF: no successor exists, so the range is
            // open-ended.
            false => {
                sqlx::query_scalar("SELECT key FROM kv WHERE col = ? AND key >= ? ORDER BY key")
                    .bind(col as u8)
                    .bind(prefix)
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        Ok(keys)
    }
}

/// Bump `prefix` to the first key that no longer starts with it.
/// `false` when there is none.
pub(super) fn next_prefix(prefix: &mut Vec<u8>) -> bool {
    while let Some(last) = prefix.last_mut() {
        if *last < u8::MAX {
            *last += 1;
            return true;
        }
        prefix.pop();
    }
    false
}

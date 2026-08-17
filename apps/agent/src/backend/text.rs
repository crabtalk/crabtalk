//! [`TextSearch`] over one FTS5 table.

use crate::backend::{SqliteStorage, kv::next_prefix};
use anyhow::Result;
use store::text::{TextHit, TextIndex, TextSearch};

impl TextSearch for SqliteStorage {
    async fn index_text(
        &self,
        index: TextIndex,
        key: &[u8],
        text: &str,
        weight: f64,
    ) -> Result<()> {
        let key = String::from_utf8_lossy(key).into_owned();
        let mut tx = self.pool.begin().await?;
        // FTS5 has no upsert: re-indexing is a delete then an insert, or
        // the old text keeps matching.
        sqlx::query("DELETE FROM text_index WHERE ix = ? AND key = ?")
            .bind(index as u8)
            .bind(&key)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO text_index (body, ix, key, weight) VALUES (?, ?, ?, ?)")
            .bind(text)
            .bind(index as u8)
            .bind(&key)
            .bind(weight)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn drop_text(&self, index: TextIndex, key: &[u8]) -> Result<()> {
        sqlx::query("DELETE FROM text_index WHERE ix = ? AND key = ?")
            .bind(index as u8)
            .bind(String::from_utf8_lossy(key).into_owned())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn drop_text_prefix(&self, index: TextIndex, prefix: &[u8]) -> Result<()> {
        // A range rather than LIKE: a key can contain `%` or `_`, and
        // every key this store writes is ASCII, so bumping the last byte
        // gives the exclusive upper bound.
        let mut upper = prefix.to_vec();
        let lower = String::from_utf8_lossy(prefix).into_owned();
        match next_prefix(&mut upper) {
            true => {
                sqlx::query("DELETE FROM text_index WHERE ix = ? AND key >= ? AND key < ?")
                    .bind(index as u8)
                    .bind(&lower)
                    .bind(String::from_utf8_lossy(&upper).into_owned())
                    .execute(&self.pool)
                    .await?;
            }
            false => {
                sqlx::query("DELETE FROM text_index WHERE ix = ? AND key >= ?")
                    .bind(index as u8)
                    .bind(&lower)
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(())
    }

    async fn search_text(
        &self,
        index: TextIndex,
        query: &str,
        limit: usize,
    ) -> Result<Vec<TextHit>> {
        let matcher = fts_query(query);
        if matcher.is_empty() {
            return Ok(Vec::new());
        }
        // `bm25()` is negative-is-better and only exists in the query
        // carrying the MATCH; the sign flip here is what lets everything
        // above treat a bigger score as a better one.
        let rows: Vec<(String, f64)> = sqlx::query_as(
            "SELECT key, -(bm25(text_index) * CAST(weight AS REAL)) AS score
             FROM text_index
             WHERE text_index MATCH ?1 AND ix = ?2
             ORDER BY score DESC
             LIMIT ?3",
        )
        .bind(&matcher)
        .bind(index as u8)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(key, score)| TextHit {
                key: key.into_bytes(),
                score,
            })
            .collect())
    }
}

/// Quote a user query as FTS5 string literals so punctuation is terms
/// rather than query syntax — `foo(bar)` would otherwise be a parse
/// error rather than a search.
fn fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

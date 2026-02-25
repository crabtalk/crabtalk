//! Memory trait implementation for SqliteMemory.

use crate::SqliteMemory;
use crate::sql;
use crate::utils::now_unix;
use agent::{Embedder, Memory, MemoryEntry, RecallOptions};
use anyhow::Result;
use std::future::Future;

impl<E: Embedder> Memory for SqliteMemory<E> {
    fn get(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        let now = now_unix();
        conn.execute(sql::TOUCH_ACCESS, rusqlite::params![now as i64, key])
            .ok();
        conn.query_row(sql::SELECT_VALUE, [key], |row| row.get(0))
            .ok()
    }

    fn entries(&self) -> Vec<(String, String)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(sql::SELECT_ENTRIES).unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    fn set(&self, key: impl Into<String>, value: impl Into<String>) -> Option<String> {
        let key = key.into();
        let value = value.into();
        let conn = self.conn.lock().unwrap();
        let now = now_unix() as i64;

        let old: Option<String> = conn
            .query_row(sql::SELECT_VALUE, [&key], |row| row.get(0))
            .ok();

        conn.execute(sql::UPSERT, rusqlite::params![key, value, now])
            .ok();

        old
    }

    fn remove(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        let old: Option<String> = conn
            .query_row(sql::SELECT_VALUE, [key], |row| row.get(0))
            .ok();
        if old.is_some() {
            conn.execute(sql::DELETE, [key]).ok();
        }
        old
    }

    fn store(
        &self,
        key: impl Into<String> + Send,
        value: impl Into<String> + Send,
    ) -> impl Future<Output = Result<()>> + Send {
        let key = key.into();
        let value = value.into();

        async move {
            // Auto-embed when embedder is present.
            let embedding = if let Some(embedder) = &self.embedder {
                let emb = embedder.embed(&value).await;
                if emb.is_empty() { None } else { Some(emb) }
            } else {
                None
            };

            self.store_with_metadata(&key, &value, None, embedding.as_deref())?;
            Ok(())
        }
    }

    fn recall(
        &self,
        query: &str,
        options: RecallOptions,
    ) -> impl Future<Output = Result<Vec<MemoryEntry>>> + Send {
        let query = query.to_owned();

        async move {
            // Embed query when embedder is present.
            let query_embedding = if let Some(embedder) = &self.embedder {
                let emb = embedder.embed(&query).await;
                if emb.is_empty() { None } else { Some(emb) }
            } else {
                None
            };

            self.recall_sync(&query, &options, query_embedding.as_deref())
        }
    }

    fn compile_relevant(&self, query: &str) -> impl Future<Output = String> + Send {
        let query = query.to_owned();

        async move {
            let opts = RecallOptions {
                limit: 5,
                ..Default::default()
            };

            // Embed query when embedder is present.
            let query_embedding = if let Some(embedder) = &self.embedder {
                let emb = embedder.embed(&query).await;
                if emb.is_empty() { None } else { Some(emb) }
            } else {
                None
            };

            let entries = self
                .recall_sync(&query, &opts, query_embedding.as_deref())
                .unwrap_or_default();

            if entries.is_empty() {
                return String::new();
            }

            let mut out = String::from("<memory>\n");
            for entry in &entries {
                out.push_str(&format!("<{}>\n", entry.key));
                out.push_str(&entry.value);
                if !entry.value.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&format!("</{}>\n", entry.key));
            }
            out.push_str("</memory>");
            out
        }
    }
}

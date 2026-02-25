//! SQLite-backed memory for Walrus agents.
//!
//! Provides [`SqliteMemory`], a persistent [`Memory`](agent::Memory) implementation
//! using SQLite with FTS5 full-text search and optional hybrid vector recall.
//!
//! All SQL lives in `sql/*.sql` files, loaded via `include_str!`.

pub use crate::utils::cosine_similarity;
use crate::utils::{decode_embedding, mmr_rerank, now_unix};
use agent::{Embedder, MemoryEntry, RecallOptions};
use anyhow::Result;
use compact_str::CompactString;
use rusqlite::Connection;
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::Mutex};

mod memory;
mod sql;
mod utils;

/// SQLite-backed memory store with optional embedding support.
///
/// Wraps a `rusqlite::Connection` in a `Mutex` for thread safety.
/// Generic over `E: Embedder` for optional vector search.
pub struct SqliteMemory<E: Embedder> {
    conn: Mutex<Connection>,
    embedder: Option<E>,
}

impl<E: Embedder> SqliteMemory<E> {
    /// Open or create a SQLite database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let mem = Self {
            conn: Mutex::new(conn),
            embedder: None,
        };
        mem.init_schema()?;
        Ok(mem)
    }

    /// Create an in-memory database (useful for testing).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let mem = Self {
            conn: Mutex::new(conn),
            embedder: None,
        };
        mem.init_schema()?;
        Ok(mem)
    }

    /// Attach an embedder for vector search.
    pub fn with_embedder(mut self, embedder: E) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Initialize the database schema.
    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(sql::SCHEMA)?;
        Ok(())
    }

    /// Execute the recall pipeline synchronously.
    ///
    /// 1. BM25 via FTS5 MATCH (under lock)
    /// 2. Vector scan (under lock, if embeddings requested)
    /// 3. Lock released — scoring, RRF fusion, MMR done without lock
    fn recall_sync(
        &self,
        query: &str,
        options: &RecallOptions,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<MemoryEntry>> {
        let now = now_unix();
        let limit = if options.limit == 0 {
            10
        } else {
            options.limit
        };

        // Phase 1: DB queries under lock. Collect raw rows, release lock.
        let (bm25_candidates, vec_candidates) = {
            let conn = self.conn.lock().unwrap();

            // BM25 path: FTS5 MATCH.
            let mut fts_stmt = conn.prepare(sql::RECALL_FTS)?;
            let bm25: Vec<(MemoryEntry, f64)> = fts_stmt
                .query_map([query], |row| {
                    let emb_blob: Option<Vec<u8>> = row.get(6)?;
                    Ok(MemoryEntry {
                        key: CompactString::new(row.get::<_, String>(0)?),
                        value: row.get(1)?,
                        metadata: row
                            .get::<_, Option<String>>(2)?
                            .and_then(|s| serde_json::from_str(&s).ok()),
                        created_at: row.get::<_, i64>(3)? as u64,
                        accessed_at: row.get::<_, i64>(4)? as u64,
                        access_count: row.get::<_, i32>(5)? as u32,
                        embedding: emb_blob.map(|b| decode_embedding(&b)),
                    })
                    .map(|entry| (entry, row.get::<_, f64>(7).unwrap_or(0.0)))
                })?
                .filter_map(|r| r.ok())
                .collect();

            // Vector path (only if query embedding provided).
            let vec = if query_embedding.is_some() {
                let mut vec_stmt = conn.prepare(sql::RECALL_VECTOR)?;
                vec_stmt
                    .query_map([], |row| {
                        let emb_blob: Option<Vec<u8>> = row.get(6)?;
                        Ok(MemoryEntry {
                            key: CompactString::new(row.get::<_, String>(0)?),
                            value: row.get(1)?,
                            metadata: row
                                .get::<_, Option<String>>(2)?
                                .and_then(|s| serde_json::from_str(&s).ok()),
                            created_at: row.get::<_, i64>(3)? as u64,
                            accessed_at: row.get::<_, i64>(4)? as u64,
                            access_count: row.get::<_, i32>(5)? as u32,
                            embedding: emb_blob.map(|b| decode_embedding(&b)),
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };

            (bm25, vec)
            // conn lock dropped here
        };

        // Phase 2: Scoring and fusion (no lock held).

        // Temporal decay: score * e^(-lambda * age_days), half-life 30 days.
        let lambda = std::f64::consts::LN_2 / 30.0;
        let bm25_scored: Vec<(MemoryEntry, f64)> = bm25_candidates
            .into_iter()
            .map(|(entry, bm25_rank)| {
                let bm25_score = -bm25_rank;
                let age_days = now.saturating_sub(entry.accessed_at) as f64 / 86400.0;
                let decay = (-lambda * age_days).exp();
                (entry, bm25_score * decay)
            })
            .collect();

        let scored = if let Some(q_emb) = query_embedding {
            // Compute cosine similarity for vector candidates.
            let vec_scored: Vec<(MemoryEntry, f64)> = vec_candidates
                .into_iter()
                .filter_map(|entry| {
                    let sim = entry
                        .embedding
                        .as_ref()
                        .map(|e| cosine_similarity(e, q_emb))
                        .unwrap_or(0.0);
                    if sim > 0.0 { Some((entry, sim)) } else { None }
                })
                .collect();

            // RRF fusion: score = 1/(k + rank_bm25) + 1/(k + rank_vector), k=60.
            // Borrowed-key HashMaps for O(1) rank lookup, no key cloning.
            let k = 60.0_f64;

            let mut bm25_ranked = bm25_scored;
            bm25_ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            let mut vec_ranked = vec_scored;
            vec_ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Compute RRF scores while vecs are borrowed, then drain entries.
            let rrf_scores: Vec<f64>;
            let bm25_in_vec: Vec<bool>;
            {
                let vec_rank_map: HashMap<&str, usize> = vec_ranked
                    .iter()
                    .enumerate()
                    .map(|(i, (e, _))| (e.key.as_str(), i + 1))
                    .collect();
                let bm25_key_set: HashMap<&str, ()> = bm25_ranked
                    .iter()
                    .map(|(e, _)| (e.key.as_str(), ()))
                    .collect();

                // Score BM25 entries (index = rank).
                rrf_scores = bm25_ranked
                    .iter()
                    .enumerate()
                    .map(|(i, (e, _))| {
                        1.0 / (k + (i + 1) as f64)
                            + vec_rank_map
                                .get(e.key.as_str())
                                .map(|&r| 1.0 / (k + r as f64))
                                .unwrap_or(0.0)
                    })
                    .collect();

                // Mark which vec entries are also in bm25 (for dedup).
                bm25_in_vec = vec_ranked
                    .iter()
                    .map(|(e, _)| bm25_key_set.contains_key(e.key.as_str()))
                    .collect();
                // borrowed maps dropped here
            }

            // Drain entries and pair with scores.
            let mut fused = Vec::with_capacity(bm25_ranked.len() + vec_ranked.len());
            for (score, (entry, _)) in rrf_scores.into_iter().zip(bm25_ranked) {
                fused.push((entry, score));
            }
            for (i, (entry, _)) in vec_ranked.into_iter().enumerate() {
                if bm25_in_vec[i] {
                    continue;
                }
                fused.push((entry, 1.0 / (k + (i + 1) as f64)));
            }
            fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            fused
        } else {
            bm25_scored
        };

        if scored.is_empty() {
            return Ok(Vec::new());
        }

        // Phase 3: Filters and MMR (no lock held).
        let mut filtered = scored;
        if let Some((start, end)) = options.time_range {
            filtered.retain(|(entry, _)| entry.created_at >= start && entry.created_at <= end);
        }
        if let Some(threshold) = options.relevance_threshold {
            filtered.retain(|(_, score)| *score >= threshold as f64);
        }
        if filtered.is_empty() {
            return Ok(Vec::new());
        }

        let use_cosine = query_embedding.is_some();
        Ok(mmr_rerank(filtered, limit, 0.7, use_cosine))
    }

    /// Store a key-value pair with optional metadata and embedding.
    pub fn store_with_metadata(
        &self,
        key: &str,
        value: &str,
        metadata: Option<&Value>,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = now_unix() as i64;
        let meta_json = metadata.map(|m| serde_json::to_string(m).unwrap());
        let emb_blob: Option<Vec<u8>> =
            embedding.map(|e| e.iter().flat_map(|f| f.to_le_bytes()).collect());

        conn.execute(
            sql::UPSERT_FULL,
            rusqlite::params![key, value, meta_json, now, emb_blob],
        )?;
        Ok(())
    }

    /// Get a full MemoryEntry for a key.
    pub fn get_entry(&self, key: &str) -> Option<MemoryEntry> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(sql::SELECT_ENTRY, [key], |row| {
            let emb_blob: Option<Vec<u8>> = row.get(6)?;
            Ok(MemoryEntry {
                key: CompactString::new(row.get::<_, String>(0)?),
                value: row.get(1)?,
                metadata: row
                    .get::<_, Option<String>>(2)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                created_at: row.get::<_, i64>(3)? as u64,
                accessed_at: row.get::<_, i64>(4)? as u64,
                access_count: row.get::<_, i32>(5)? as u32,
                embedding: emb_blob.map(|b| decode_embedding(&b)),
            })
        })
        .ok()
    }
}

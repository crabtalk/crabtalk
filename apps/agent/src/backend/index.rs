//! [`SqlIndex`] over the query tables.
//!
//! Every method here answers one named question. None of them returns a
//! body: an agent config, a skill's markdown, a memory entry and a
//! session's messages are all KV reads the caller makes for the handful
//! of keys a query hands back.

use crate::backend::{SqliteStorage, convert};
use anyhow::Result;
use sqlx::Row;
use std::str::FromStr;
use store::{
    AgentId, SkillSummary,
    session::{SearchOptions, SessionHit},
    session::{SessionHandle, SessionMeta},
    sql::{MessageDoc, SqlIndex},
};

impl SqlIndex for SqliteStorage {
    // ── Agents ─────────────────────────────────────────────────────

    async fn index_agent(&self, id: &AgentId, name: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO agents (id, name) VALUES (?, ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name",
        )
        .bind(id.to_string())
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn unindex_agent(&self, id: &AgentId) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn agent_ids(&self) -> Result<Vec<AgentId>> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM agents ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(ids
            .iter()
            .filter_map(|id| AgentId::from_str(id).ok())
            .collect())
    }

    async fn agent_id_by_name(&self, name: &str) -> Result<Option<AgentId>> {
        let id: Option<String> = sqlx::query_scalar("SELECT id FROM agents WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(id.and_then(|id| AgentId::from_str(&id).ok()))
    }

    async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        let updated = sqlx::query("UPDATE agents SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(updated > 0)
    }

    // ── Sessions ───────────────────────────────────────────────────

    async fn index_session(&self, handle: &SessionHandle, meta: &SessionMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions
                 (handle, agent, created_by, title, created_at, updated_at,
                  message_count, summary)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(handle) DO UPDATE SET
                 title         = excluded.title,
                 updated_at    = excluded.updated_at,
                 message_count = excluded.message_count,
                 summary       = excluded.summary",
        )
        .bind(handle.as_str())
        .bind(meta.agent.to_string())
        .bind(&meta.created_by)
        .bind(&meta.title)
        .bind(&meta.created_at)
        .bind(&meta.updated_at)
        .bind(meta.message_count as i64)
        .bind(meta.summary.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn unindex_session(&self, handle: &SessionHandle) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM sessions WHERE handle = ?")
            .bind(handle.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected();
        sqlx::query("DELETE FROM session_search WHERE session_handle = ?")
            .bind(handle.as_str())
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM session_meta_search WHERE session_handle = ?")
            .bind(handle.as_str())
            .execute(&self.pool)
            .await?;
        Ok(deleted > 0)
    }

    async fn latest_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        let handle: Option<String> = sqlx::query_scalar(
            "SELECT handle FROM sessions
             WHERE agent = ? AND created_by = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(agent.to_string())
        .bind(created_by)
        .fetch_optional(&self.pool)
        .await?;
        Ok(handle.map(SessionHandle::new))
    }

    async fn session_rows(&self) -> Result<Vec<(SessionHandle, SessionMeta)>> {
        let rows = sqlx::query(
            "SELECT handle, agent, created_by, title, created_at, updated_at,
                    message_count, summary
             FROM sessions
             ORDER BY updated_at DESC, created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let handle = SessionHandle::new(row.try_get::<String, _>("handle")?);
            out.push((handle, convert::meta(&row)?));
        }
        Ok(out)
    }

    async fn session_handles_of(&self, agent: &AgentId) -> Result<Vec<SessionHandle>> {
        let handles: Vec<String> =
            sqlx::query_scalar("SELECT handle FROM sessions WHERE agent = ?")
                .bind(agent.to_string())
                .fetch_all(&self.pool)
                .await?;
        Ok(handles.into_iter().map(SessionHandle::new).collect())
    }

    // ── Messages ───────────────────────────────────────────────────

    async fn index_messages(&self, handle: &SessionHandle, docs: &[MessageDoc]) -> Result<()> {
        if docs.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for doc in docs {
            sqlx::query(
                "INSERT INTO session_search (body, session_handle, idx, role)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&doc.body)
            .bind(handle.as_str())
            .bind(doc.idx as i64)
            .bind(&doc.role)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn drop_messages_from(&self, handle: &SessionHandle, keep: usize) -> Result<()> {
        sqlx::query("DELETE FROM session_search WHERE session_handle = ? AND idx >= ?")
            .bind(handle.as_str())
            .bind(keep as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn search_messages(&self, query: &str, opts: &SearchOptions) -> Result<Vec<SessionHit>> {
        self.search_sessions(query, opts).await
    }

    // ── Memory ─────────────────────────────────────────────────────

    async fn index_memory(&self, name: &str, content: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        // FTS5 has no upsert: a re-indexed entry is a delete then an
        // insert, or the old text keeps matching.
        sqlx::query("DELETE FROM memory_search WHERE name = ?")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO memory_search (body, name) VALUES (?, ?)")
            .bind(content)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO memory_index (name) VALUES (?)")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn unindex_memory(&self, name: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM memory_search WHERE name = ?")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        let deleted = sqlx::query("DELETE FROM memory_index WHERE name = ?")
            .bind(name)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    async fn search_memory(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let matcher = crate::backend::search::fts_query(query);
        if matcher.is_empty() {
            return Ok(Vec::new());
        }
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM memory_search
             WHERE memory_search MATCH ?
             ORDER BY bm25(memory_search) LIMIT ?",
        )
        .bind(&matcher)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(names)
    }

    async fn memory_names(&self) -> Result<Vec<String>> {
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM memory_index ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        Ok(names)
    }

    // ── Skills ─────────────────────────────────────────────────────

    async fn index_skill(&self, summary: &SkillSummary) -> Result<()> {
        sqlx::query(
            "INSERT INTO skills_index (name, description) VALUES (?, ?)
             ON CONFLICT(name) DO UPDATE SET description = excluded.description",
        )
        .bind(&summary.name)
        .bind(&summary.description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn unindex_skill(&self, name: &str) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM skills_index WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted > 0)
    }

    async fn skill_summaries(&self, limit: usize, offset: usize) -> Result<Vec<SkillSummary>> {
        let rows = sqlx::query(
            "SELECT name, description FROM skills_index
             ORDER BY name LIMIT ? OFFSET ?",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(SkillSummary {
                name: row.try_get("name")?,
                description: row.try_get("description")?,
            });
        }
        Ok(out)
    }
}

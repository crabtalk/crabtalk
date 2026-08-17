//! Session persistence — one row per thread, plus the message and event
//! streams keyed to it.

use crate::backend::sqlite::{SqliteStorage, convert, schema::BEGIN_IMMEDIATE};
use crate::{
    AgentId, HistoryEntry,
    storage::{EventLine, SessionHandle, SessionMeta, SessionSnapshot, SessionSummary},
};
use anyhow::Result;
use sqlx::Row;

impl SqliteStorage {
    pub(super) async fn create_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> Result<SessionHandle> {
        // Opaque identity: the handle encodes
        // nothing, so renaming an agent never orphans its transcripts.
        let handle = ulid::Ulid::new().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (handle, agent, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&handle)
        .bind(agent.to_string())
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(SessionHandle::new(handle))
    }

    /// The most recent session for an `(agent, created_by)` pair — the
    /// query the `sessions_agent_creator` index exists for.
    pub(super) async fn find_latest_session(
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

    pub(super) async fn load_session(
        &self,
        handle: &SessionHandle,
    ) -> Result<Option<SessionSnapshot>> {
        let h = handle.as_str();
        let Some(row) = sqlx::query(
            "SELECT agent, created_by, title, created_at, updated_at,
                    message_count, summary, archive
             FROM sessions WHERE handle = ?",
        )
        .bind(h)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        let meta = convert::meta(&row)?;
        let archive: Option<String> = row.try_get("archive")?;
        let entry_jsons: Vec<String> = sqlx::query_scalar(
            "SELECT entry_json FROM session_messages
             WHERE session_handle = ? ORDER BY idx",
        )
        .bind(h)
        .fetch_all(&self.pool)
        .await?;

        // Rows that survive a compact are the turns since it, so the
        // archive plus what's left is the whole session.
        let mut history = Vec::with_capacity(entry_jsons.len());
        for json in &entry_jsons {
            match serde_json::from_str::<HistoryEntry>(json) {
                Ok(entry) => history.push(entry),
                Err(e) => tracing::warn!("skipping unreadable session message: {e}"),
            }
        }
        Ok(Some(SessionSnapshot {
            meta,
            history,
            archive,
        }))
    }

    pub(super) async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
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
            out.push(SessionSummary {
                handle: SessionHandle::new(row.try_get::<String, _>("handle")?),
                meta: convert::meta(&row)?,
            });
        }
        Ok(out)
    }

    pub(super) async fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let h = handle.as_str();
        let mut tx = self.pool.begin().await?;
        sqlx::query(BEGIN_IMMEDIATE).execute(&mut *tx).await.ok();
        let next: i64 = next_idx(&mut tx, "session_messages", h).await?;
        for (offset, entry) in entries.iter().enumerate() {
            sqlx::query(
                "INSERT INTO session_messages (session_handle, idx, entry_json)
                 VALUES (?, ?, ?)",
            )
            .bind(h)
            .bind(next + offset as i64)
            .bind(serde_json::to_string(entry)?)
            .execute(&mut *tx)
            .await?;
            // Indexed in the same transaction as the write, so an append
            // that is not searchable is not a state this can reach.
            if let Some((body, role)) = super::search::indexable(entry) {
                sqlx::query(
                    "INSERT INTO session_search (body, session_handle, idx, role)
                     VALUES (?, ?, ?, ?)",
                )
                .bind(body)
                .bind(h)
                .bind(next + offset as i64)
                .bind(role)
                .execute(&mut *tx)
                .await?;
            }
        }
        sqlx::query(
            "UPDATE sessions
             SET message_count = (SELECT COUNT(*) FROM session_messages WHERE session_handle = ?),
                 updated_at = ?
             WHERE handle = ?",
        )
        .bind(h)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(h)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let h = handle.as_str();
        let mut tx = self.pool.begin().await?;
        let next: i64 = next_idx(&mut tx, "session_events", h).await?;
        for (offset, event) in events.iter().enumerate() {
            let (kind, ts) = convert::kind_and_ts(event);
            sqlx::query(
                "INSERT INTO session_events (session_handle, idx, ts, kind, payload_json)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(h)
            .bind(next + offset as i64)
            .bind(ts)
            .bind(kind)
            .bind(serde_json::to_string(event)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Record the archive holding everything up to now, and drop the
    /// messages it replaces. The archive is the pre-compact record, so
    /// leaving the rows would double-count them on the next load.
    pub(super) async fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive_name: &str,
    ) -> Result<()> {
        let h = handle.as_str();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE sessions SET archive = ?, message_count = 0, updated_at = ? WHERE handle = ?",
        )
        .bind(archive_name)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(h)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM session_messages WHERE session_handle = ?")
            .bind(h)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn truncate_session_messages(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> Result<()> {
        let h = handle.as_str();
        let mut tx = self.pool.begin().await?;
        // `idx` is append-only and never renumbered, so "keep the first
        // N" is a rank over the ordering rather than `idx < keep`.
        sqlx::query(
            "DELETE FROM session_messages
             WHERE session_handle = ?1
               AND idx NOT IN (
                   SELECT idx FROM session_messages
                   WHERE session_handle = ?1 ORDER BY idx LIMIT ?2
               )",
        )
        .bind(h)
        .bind(keep as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE sessions
             SET message_count = (SELECT COUNT(*) FROM session_messages WHERE session_handle = ?),
                 updated_at = ?
             WHERE handle = ?",
        )
        .bind(h)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(h)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &SessionMeta,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions
             SET agent = ?, created_by = ?, title = ?, created_at = ?,
                 updated_at = ?, message_count = ?, summary = ?
             WHERE handle = ?",
        )
        .bind(meta.agent.to_string())
        .bind(&meta.created_by)
        .bind(&meta.title)
        .bind(&meta.created_at)
        .bind(&meta.updated_at)
        .bind(meta.message_count as i64)
        .bind(meta.summary.as_deref())
        .bind(handle.as_str())
        .execute(&self.pool)
        .await?;
        // Title and summary rank the session as a whole; rewrite its one
        // search document rather than trying to patch it.
        sqlx::query("DELETE FROM session_meta_search WHERE session_handle = ?")
            .bind(handle.as_str())
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO session_meta_search (title, summary, session_handle)
             VALUES (?, ?, ?)",
        )
        .bind(&meta.title)
        .bind(meta.summary.as_deref().unwrap_or(""))
        .bind(handle.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every session of one agent, dropped in one pass. The FTS5 tables
    /// are virtual, so their rows are deleted by handle rather than
    /// cascading off the `sessions` row.
    pub(super) async fn delete_sessions_of(&self, agent: &AgentId) -> Result<usize> {
        let handles: Vec<String> =
            sqlx::query_scalar("SELECT handle FROM sessions WHERE agent = ?")
                .bind(agent.to_string())
                .fetch_all(&self.pool)
                .await?;
        let mut removed = 0;
        for handle in handles {
            if self.delete_session(&SessionHandle::new(handle)).await? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub(super) async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        let h = handle.as_str();
        // Messages and events cascade on the FK. The FTS5 tables are
        // virtual, so they have no foreign key to cascade along.
        for table in ["session_search", "session_meta_search"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE session_handle = ?"))
                .bind(h)
                .execute(&self.pool)
                .await?;
        }
        let done = sqlx::query("DELETE FROM sessions WHERE handle = ?")
            .bind(h)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}

/// Next append position for a session's stream.
async fn next_idx(
    tx: &mut sqlx::SqliteConnection,
    table: &str,
    handle: &str,
) -> Result<i64, sqlx::Error> {
    // `table` is a literal from the two call sites, never user input.
    let sql = format!("SELECT COALESCE(MAX(idx) + 1, 0) FROM {table} WHERE session_handle = ?");
    sqlx::query_scalar(&sql).bind(handle).fetch_one(tx).await
}

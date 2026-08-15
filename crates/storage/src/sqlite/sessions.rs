//! Session persistence — one row per thread, plus the message and event
//! streams keyed to it.

use crate::sqlite::{SqliteStorage, schema::BEGIN_IMMEDIATE};
use anyhow::Result;
use sqlx::Row;
use wcore::{
    model::HistoryEntry,
    storage::{ConversationMeta, EventLine, SessionHandle, SessionSnapshot, SessionSummary},
};

impl SqliteStorage {
    pub(super) async fn create_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> Result<SessionHandle> {
        // Opaque identity, matching `FsStorage`: the handle encodes
        // nothing, so renaming an agent never orphans its transcripts.
        let handle = ulid::Ulid::new().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (handle, agent, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&handle)
        .bind(agent)
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
        agent: &str,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        let handle: Option<String> = sqlx::query_scalar(
            "SELECT handle FROM sessions
             WHERE agent = ? AND created_by = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(agent)
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

        let meta = meta_from_row(&row)?;
        let archive: Option<String> = row.try_get("archive")?;
        let entry_jsons: Vec<String> = sqlx::query_scalar(
            "SELECT entry_json FROM session_messages
             WHERE session_handle = ? ORDER BY idx",
        )
        .bind(h)
        .fetch_all(&self.pool)
        .await?;

        // Rows that survive a compact are the turns since it, so the
        // archive plus what's left is the whole conversation.
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
                meta: meta_from_row(&row)?,
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
            let (kind, ts) = kind_and_ts(event);
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
        meta: &ConversationMeta,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions
             SET agent = ?, created_by = ?, title = ?, created_at = ?,
                 updated_at = ?, message_count = ?, summary = ?
             WHERE handle = ?",
        )
        .bind(&meta.agent)
        .bind(&meta.created_by)
        .bind(&meta.title)
        .bind(&meta.created_at)
        .bind(&meta.updated_at)
        .bind(meta.message_count as i64)
        .bind(meta.summary.as_deref())
        .bind(handle.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(super) async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        // Messages and events go with it: both cascade on the FK.
        let done = sqlx::query("DELETE FROM sessions WHERE handle = ?")
            .bind(handle.as_str())
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

fn meta_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ConversationMeta> {
    Ok(ConversationMeta {
        agent: row.try_get("agent")?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        title: row.try_get("title")?,
        updated_at: row.try_get("updated_at")?,
        message_count: row.try_get::<i64, _>("message_count")? as u64,
        summary: row.try_get("summary")?,
    })
}

/// Discriminator and timestamp for an event row. These mirror
/// `EventLine`'s `#[serde(tag = "event", rename_all = "snake_case")]`, so
/// `WHERE kind = 'done'` selects exactly the rows whose payload carries
/// that tag. The match is exhaustive on purpose: a new variant should
/// fail to compile here rather than land under a wrong `kind`.
fn kind_and_ts(event: &EventLine) -> (&'static str, &str) {
    match event {
        EventLine::ToolStart { ts, .. } => ("tool_start", ts),
        EventLine::ToolResult { ts, .. } => ("tool_result", ts),
        EventLine::Done { ts, .. } => ("done", ts),
        EventLine::UserSteered { ts, .. } => ("user_steered", ts),
    }
}

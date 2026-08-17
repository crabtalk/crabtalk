//! Row and payload conversions.
//!
//! These can't be `From` impls: `SqliteRow` belongs to sqlx and
//! `ConversationMeta` to core, so neither is local here. Grouping them in
//! one module named for what they produce is the next best thing —
//! `convert::meta(&row)` reads like the trait would.

use anyhow::Result;
use schema::storage::{ConversationMeta, EventLine};
use sqlx::{Row, sqlite::SqliteRow};

/// A session row's metadata.
pub(super) fn meta(row: &SqliteRow) -> Result<ConversationMeta> {
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
pub(super) fn kind_and_ts(event: &EventLine) -> (&'static str, &str) {
    match event {
        EventLine::ToolStart { ts, .. } => ("tool_start", ts),
        EventLine::ToolResult { ts, .. } => ("tool_result", ts),
        EventLine::Done { ts, .. } => ("done", ts),
        EventLine::UserSteered { ts, .. } => ("user_steered", ts),
    }
}

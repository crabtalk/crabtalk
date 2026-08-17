//! Row and payload conversions.
//!
//! These can't be `From` impls: `SqliteRow` belongs to sqlx and
//! `SessionMeta` to core, so neither is local here. Grouping them in
//! one module named for what they produce is the next best thing —
//! `convert::meta(&row)` reads like the trait would.

use anyhow::Result;
use sqlx::{Row, sqlite::SqliteRow};
use std::str::FromStr;
use store::{AgentId, SessionMeta};

/// A session row's metadata.
pub(super) fn meta(row: &SqliteRow) -> Result<SessionMeta> {
    Ok(SessionMeta {
        agent: AgentId::from_str(row.try_get("agent")?)?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        title: row.try_get("title")?,
        updated_at: row.try_get("updated_at")?,
        message_count: row.try_get::<i64, _>("message_count")? as u64,
        summary: row.try_get("summary")?,
    })
}

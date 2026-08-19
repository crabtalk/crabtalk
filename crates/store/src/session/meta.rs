//! A session's identity and metadata.

use crate::{AgentId, session::history::HistoryEntry};
use serde::{Deserialize, Serialize};

/// Opaque handle identifying a persisted session.
///
/// It encodes nothing — not the agent, not the sender, not a date — so
/// renaming an agent never orphans its transcripts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionHandle(String);

impl SessionHandle {
    /// Construct a handle from a store-assigned identifier.
    pub fn new(slug: impl Into<String>) -> Self {
        Self(slug.into())
    }

    /// The raw identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Metadata persisted alongside a session's messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub agent: AgentId,
    pub created_by: String,
    pub created_at: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub message_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Meta plus working-context history, already replayed past the last
/// compact marker. Returned by
/// [`Sessions::load_session`](crate::interface::Sessions::load_session).
pub struct SessionSnapshot {
    pub meta: SessionMeta,
    pub history: Vec<HistoryEntry>,
    /// Names the memory entry holding the compacted prefix, if the
    /// session has been compacted. The text itself is never here.
    pub archive: Option<String>,
}

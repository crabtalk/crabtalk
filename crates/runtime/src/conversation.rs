//! Conversation — pure working-context container.

use crate::ConversationHandle;
use std::time::Instant;
use wcore::{model::HistoryEntry, storage::ConversationMeta};

/// A conversation tied to a specific agent.
///
/// Pure working-context container. Persistence is delegated to the
/// Storage trait via the session handle.
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Unique conversation identifier (monotonic counter, runtime-only).
    pub id: u64,
    /// Conversation history (the working context for the LLM).
    pub history: Vec<HistoryEntry>,
    /// Conversation title (set by the `set_title` tool).
    pub title: String,
    /// When this conversation was loaded/created in this process.
    /// Process-local — resets across restarts.
    pub created_at: Instant,
    /// Persisted RFC3339 creation timestamp. Populated at construction
    /// and overwritten on resume from `ConversationMeta.created_at`;
    /// never bumped after that.
    pub created_at_iso: String,
    /// Latest compaction summary, written by overflow compaction and
    /// contributed to session search ranking (3× boost). `None` until
    /// the first compaction.
    pub summary: Option<String>,
    /// Persistent conversation identity, assigned by the storage layer.
    /// `None` until the first persistence call.
    pub handle: Option<ConversationHandle>,
}

impl Conversation {
    /// Create a new conversation with an empty history.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            history: Vec::new(),
            title: String::new(),
            created_at: Instant::now(),
            created_at_iso: chrono::Utc::now().to_rfc3339(),
            summary: None,
            handle: None,
        }
    }

    /// Build a [`ConversationMeta`] snapshot from this conversation's
    /// current state. `created_at` is sourced from the persisted ISO
    /// string (immutable across writes); `updated_at` is stamped now;
    /// `message_count` reflects the current history length.
    pub fn meta(&self, agent: &str, created_by: &str) -> ConversationMeta {
        ConversationMeta {
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            created_at: self.created_at_iso.clone(),
            title: self.title.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            message_count: self.history.len() as u64,
            summary: self.summary.clone(),
        }
    }
}

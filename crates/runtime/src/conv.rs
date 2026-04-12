//! Conversation — pure working-context container.

use std::time::Instant;
use wcore::{
    model::HistoryEntry,
    storage::{ConversationMeta, SessionHandle},
};

/// A conversation tied to a specific agent.
///
/// Pure working-context container. Persistence is delegated to the
/// Storage trait via the session handle.
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Unique conversation identifier (monotonic counter, runtime-only).
    pub id: u64,
    /// Name of the agent this conversation is bound to.
    pub agent: String,
    /// Conversation history (the working context for the LLM).
    pub history: Vec<HistoryEntry>,
    /// Origin of this conversation (e.g. "user", "tg:12345").
    pub created_by: String,
    /// Conversation title (set by the `set_title` tool).
    pub title: String,
    /// Accumulated active time in seconds.
    pub uptime_secs: u64,
    /// When this conversation was loaded/created in this process.
    pub created_at: Instant,
    /// Persistent session identity, assigned by the repo. `None` until
    /// the first persistence call.
    pub handle: Option<SessionHandle>,
}

impl Conversation {
    /// Create a new conversation with an empty history.
    pub fn new(id: u64, agent: impl Into<String>, created_by: impl Into<String>) -> Self {
        Self {
            id,
            agent: agent.into(),
            history: Vec::new(),
            created_by: created_by.into(),
            title: String::new(),
            uptime_secs: 0,
            created_at: Instant::now(),
            handle: None,
        }
    }

    /// Build a [`ConversationMeta`] snapshot from this conversation's
    /// current state.
    pub fn meta(&self) -> ConversationMeta {
        ConversationMeta {
            agent: self.agent.clone(),
            created_by: self.created_by.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            title: self.title.clone(),
            uptime_secs: self.uptime_secs,
        }
    }
}

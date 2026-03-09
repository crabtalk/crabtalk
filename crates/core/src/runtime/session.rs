//! Session — lightweight history container for agent conversations.

use compact_str::CompactString;
use std::time::Instant;

use crate::model::Message;

/// A conversation session tied to a specific agent.
///
/// Sessions own the conversation history and are stored behind
/// `Arc<Mutex<Session>>` in the runtime. Multiple sessions can
/// reference the same agent — each with independent history.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier (monotonic counter).
    pub id: u64,
    /// Name of the agent this session is bound to.
    pub agent: CompactString,
    /// Conversation history (user/assistant/tool messages).
    pub history: Vec<Message>,
    /// Origin of this session (e.g. "user", "telegram:12345", agent name).
    pub created_by: CompactString,
    /// When this session was created.
    pub created_at: Instant,
}

impl Session {
    /// Create a new session with an empty history.
    pub fn new(
        id: u64,
        agent: impl Into<CompactString>,
        created_by: impl Into<CompactString>,
    ) -> Self {
        Self {
            id,
            agent: agent.into(),
            history: Vec::new(),
            created_by: created_by.into(),
            created_at: Instant::now(),
        }
    }
}

//! Bounded hit shape for session search. Field bounds are enforced
//! at construction so callers can't blow up context windows by
//! requesting larger windows than the runtime is willing to return.

use wcore::model::Role;
use wcore::storage::SessionHandle;

/// Maximum bytes per window item snippet. Long messages are truncated
/// at this boundary with `truncated = true`.
pub const MAX_SNIPPET_BYTES: usize = 1024;

/// Maximum total messages per window (context_before + match +
/// context_after, capped regardless of caller request).
pub const MAX_WINDOW_ITEMS: usize = 16;

/// Maximum hits returned per query.
pub const MAX_HITS_PER_QUERY: usize = 20;

/// Caller-tunable knobs for `Runtime::search_sessions`. All limits
/// clamp to the constants above.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub limit: usize,
    pub context_before: usize,
    pub context_after: usize,
    pub agent_filter: Option<String>,
    pub sender_filter: Option<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 5,
            context_before: 4,
            context_after: 4,
            agent_filter: None,
            sender_filter: None,
        }
    }
}

/// One result of a session search — points at the matching message
/// and carries a bounded window of surrounding context. Sessions are
/// addressed by `session_handle` (the storage slug, stable across
/// process restarts); the index's internal session id is not exposed.
#[derive(Debug, Clone)]
pub struct SessionHit {
    pub session_handle: SessionHandle,
    pub msg_idx: u32,
    pub score: f64,
    pub title: String,
    pub agent: String,
    pub sender: String,
    pub created_at: String,
    pub updated_at: String,
    pub window: Vec<WindowItem>,
}

/// One message in a hit's window. `snippet` is truncated to
/// `MAX_SNIPPET_BYTES`; longer originals set `truncated = true`.
#[derive(Debug, Clone)]
pub struct WindowItem {
    pub role: Role,
    pub msg_idx: u32,
    pub snippet: String,
    pub truncated: bool,
    /// Function name on tool-call assistants and tool-result entries.
    pub tool_name: Option<String>,
}

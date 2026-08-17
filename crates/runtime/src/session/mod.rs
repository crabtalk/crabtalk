//! Session — the live working context, and the registry of them.
//!
//! A [`Session`] here is the same entity storage persists: this is the
//! live half, storage holds the record. They share the word because they
//! are one thing at two layers, which is why nothing translates between
//! them — a session carries `storage`'s [`SessionHandle`] directly.

use std::{sync::Arc, time::Instant};
use storage::{HistoryEntry, SessionHandle, SessionMeta};
use tokio::sync::Mutex;

mod sessions;

pub use sessions::Sessions;

/// A session shared between the registry that owns it and the engine
/// that runs it. The lock is held for a whole agent run — that is what
/// serializes concurrent sends to one session.
pub type SharedSession = Arc<Mutex<Session>>;

/// A session tied to a specific agent.
///
/// Pure working-context container. Persistence is delegated to the
/// Storage trait via the session handle.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier (monotonic counter, runtime-only).
    pub id: u64,
    /// The agent this session is with. Immutable once constructed.
    pub agent: String,
    /// Who opened it. Immutable once constructed.
    pub created_by: String,
    /// Session history (the working context for the LLM).
    pub history: Vec<HistoryEntry>,
    /// Session title (set by the `set_title` tool).
    pub title: String,
    /// When this session was loaded/created in this process.
    /// Process-local — resets across restarts.
    pub created_at: Instant,
    /// Persisted RFC3339 creation timestamp. Populated at construction
    /// and overwritten on resume from `SessionMeta.created_at`;
    /// never bumped after that.
    pub created_at_iso: String,
    /// Latest compaction summary, written by overflow compaction and
    /// contributed to session search ranking (3× boost). `None` until
    /// the first compaction.
    pub summary: Option<String>,
    /// Persistent session identity, assigned by the storage layer.
    /// `None` until the first persistence call.
    pub handle: Option<SessionHandle>,
}

impl Session {
    /// Create a new session with an empty history.
    pub fn new(id: u64, agent: &str, created_by: &str) -> Self {
        Self {
            id,
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            history: Vec::new(),
            title: String::new(),
            created_at: Instant::now(),
            created_at_iso: chrono::Utc::now().to_rfc3339(),
            summary: None,
            handle: None,
        }
    }

    /// Build a [`SessionMeta`] snapshot from this session's current
    /// state. `created_at` is sourced from the persisted ISO string
    /// (immutable across writes); `updated_at` is stamped now;
    /// `message_count` reflects the current history length.
    pub fn meta(&self) -> SessionMeta {
        SessionMeta {
            agent: self.agent.clone(),
            created_by: self.created_by.clone(),
            created_at: self.created_at_iso.clone(),
            title: self.title.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            message_count: self.history.len() as u64,
            summary: self.summary.clone(),
        }
    }
}

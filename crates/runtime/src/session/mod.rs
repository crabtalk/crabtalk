//! Session — the live working context, and the registry of them.
//!
//! A [`Session`] here is the same entity storage persists: this is the
//! live half, storage holds the record. They share the word because they
//! are one thing at two layers, which is why nothing translates between
//! them — a session carries `storage`'s [`SessionHandle`] directly.

use std::{path::PathBuf, sync::Arc, time::Instant};
use store::{AgentId, HistoryEntry, SessionHandle, SessionMeta};
use tokio::sync::Mutex;
pub use {
    registry::{Live, Registry},
    sessions::Sessions,
};

mod registry;
mod sessions;

/// A session shared between the registry that owns it and the engine
/// that runs it. The lock is held for a whole agent run — that is what
/// serializes concurrent sends to one session.
pub type SharedSession = Arc<Mutex<Session>>;

/// What a run tells its tool calls about the session they belong to.
///
/// The two travel together everywhere — a dispatch that knows the id but
/// not the root would bind `fs` to the wrong subtree — so they are one
/// value rather than two parameters kept in step by hand.
#[derive(Debug, Clone)]
pub struct SessionRef {
    pub id: u64,
    pub root: Option<PathBuf>,
}

impl From<&Session> for SessionRef {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id,
            root: session.root.clone(),
        }
    }
}

/// A session tied to a specific agent.
///
/// Pure working-context container. Persistence is delegated to the
/// Storage trait via the session handle.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier (monotonic counter, runtime-only).
    pub id: u64,
    /// The agent this session is with. Immutable once constructed.
    pub agent: AgentId,
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
    /// Where this session's work happens. A `Root::Session` declaration
    /// narrows to it; every other declaration ignores it.
    pub root: Option<PathBuf>,
}

impl Session {
    /// Roughly what this session's history costs resident. What the live
    /// registry's bound is spent on, and the reason that bound is bytes:
    /// a fresh session is nothing and one at full context is megabytes,
    /// so a count of them says little about the memory they hold.
    pub fn bytes(&self) -> usize {
        self.history.iter().map(HistoryEntry::bytes).sum()
    }

    /// Create a new session with an empty history.
    pub fn new(id: u64, agent: &AgentId, created_by: &str) -> Self {
        Self {
            id,
            agent: *agent,
            created_by: created_by.to_owned(),
            history: Vec::new(),
            title: String::new(),
            created_at: Instant::now(),
            created_at_iso: chrono::Utc::now().to_rfc3339(),
            summary: None,
            handle: None,
            root: None,
        }
    }

    /// Build a [`SessionMeta`] snapshot from this session's current
    /// state. `created_at` is sourced from the persisted ISO string
    /// (immutable across writes) and `updated_at` is stamped now.
    ///
    /// `message_count` is the live history length, which counts the
    /// archive prefix and per-run framing — neither of which is a
    /// stored message. The store discards it and keeps its own.
    pub fn meta(&self) -> SessionMeta {
        SessionMeta {
            agent: self.agent,
            created_by: self.created_by.clone(),
            created_at: self.created_at_iso.clone(),
            title: self.title.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            message_count: self.history.len() as u64,
            summary: self.summary.clone(),
            root: self.root.clone(),
        }
    }
}

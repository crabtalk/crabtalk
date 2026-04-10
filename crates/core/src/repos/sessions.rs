//! Session repository trait and domain types.
//!
//! [`SessionRepo`] abstracts conversation persistence. The repo owns
//! its own internal layout (step files, counters, slug assignment) —
//! callers interact via domain types only.

use crate::{
    model::HistoryEntry,
    runtime::conversation::{ArchiveSegment, ConversationMeta, EventLine},
};
use anyhow::Result;

/// Opaque handle identifying a persisted session. Created by the repo
/// on `create`, returned by `find_latest`. Callers pass it back to
/// append/load methods without interpreting the inner value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionHandle(String);

impl SessionHandle {
    /// Construct a handle from a repo-assigned identifier.
    pub fn new(slug: impl Into<String>) -> Self {
        Self(slug.into())
    }

    /// The raw identifier. Implementations use this to resolve to their
    /// internal layout.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Snapshot returned by [`SessionRepo::load`] — meta + working-context
/// history, already replayed past the last compact marker.
pub struct SessionSnapshot {
    pub meta: ConversationMeta,
    pub history: Vec<HistoryEntry>,
}

/// Summary returned by [`SessionRepo::list`] for enumeration.
pub struct SessionSummary {
    pub handle: SessionHandle,
    pub meta: ConversationMeta,
}

/// Persistence backend for conversation sessions.
///
/// Implementations own slug assignment, step-level persistence, compact
/// replay, and counter recovery. The trait speaks domain types only —
/// no step keys, no key formatting, no directory walks.
pub trait SessionRepo: Send + Sync + 'static {
    /// Create a new session. Returns an opaque handle.
    fn create(&self, agent: &str, created_by: &str) -> Result<SessionHandle>;

    /// Find the latest session for an (agent, created_by) identity.
    fn find_latest(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>>;

    /// Load a session's meta and working-context history. History is
    /// already replayed from the last compact marker forward.
    fn load(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>>;

    /// Load all archive segments (compact markers) for a session.
    fn load_archives(&self, handle: &SessionHandle) -> Result<Vec<ArchiveSegment>>;

    /// List all sessions.
    fn list_sessions(&self) -> Result<Vec<SessionSummary>>;

    /// Append history entries to a session. Auto-injected entries are
    /// skipped by the caller before calling this method.
    fn append_messages(&self, handle: &SessionHandle, entries: &[HistoryEntry]) -> Result<()>;

    /// Append trace event entries.
    fn append_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()>;

    /// Append a compact marker (archive boundary).
    fn append_compact(&self, handle: &SessionHandle, summary: &str) -> Result<()>;

    /// Overwrite session metadata (title, uptime, etc.).
    fn update_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()>;

    /// Delete a session entirely.
    fn delete(&self, handle: &SessionHandle) -> Result<bool>;
}

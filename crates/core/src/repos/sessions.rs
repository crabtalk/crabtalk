//! Session domain types.

use crate::{model::HistoryEntry, runtime::conversation::ConversationMeta};

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

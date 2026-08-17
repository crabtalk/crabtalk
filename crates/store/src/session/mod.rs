//! What a session is, as persisted.
//!
//! [`meta`] is its identity and metadata, [`history`] the turns the model
//! replays, [`event`] the trace of what happened during a run, and
//! [`SessionHit`] the shape a search result comes back in. Finding a
//! match is
//! not a backend's business: it is BM25 over the same keyspace, written
//! once in [`text`](crate::text). See RFC 0207.

pub mod event;
pub mod history;
pub mod meta;

mod hit;

pub use event::{EventLine, ToolCallTrace};
pub use history::HistoryEntry;
pub use hit::{
    MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHit, WindowItem,
};
pub use meta::{SessionHandle, SessionMeta, SessionSnapshot};

/// Sanitize a string into a slug safe to use inside a name.
pub fn sender_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

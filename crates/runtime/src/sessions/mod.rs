//! Session search — BM25 over conversation messages.
//!
//! Returns bounded windowed excerpts so callers (typically agent
//! tools) can surface matches without paying the cost of a full
//! session load. See RFC 0185 for the design.

mod hit;
mod index;

pub use hit::{
    MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHit, WindowItem,
};
pub use index::{MessageRef, SessionIndex};

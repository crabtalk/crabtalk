//! The shape of a session-search result.
//!
//! Bounded windowed excerpts, so a caller can surface a match without
//! paying for a full session load. How a backend finds the match is its
//! own business — sqlite uses FTS5. See RFC 0185 for the design.

pub mod history;
mod hit;

pub use hit::{
    MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHit, WindowItem,
};

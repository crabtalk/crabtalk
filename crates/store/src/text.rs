//! The text primitive — ranked full-text search.
//!
//! The one thing keys cannot do. Ordered lookups, set membership and
//! name resolution are all secondary indexes, and a secondary index is
//! just more keys in [`KVStorage`](crate::KVStorage); ranking a body of
//! prose against a query is not.
//!
//! So this trait is three operations, and it knows nothing about what it
//! is indexing. It takes bytes for a key, a string to index, and a
//! number to weight by — a caller that wants a user's own words to
//! outrank a tool's passes a bigger weight, and what a "role" is stays
//! where it belongs.

use anyhow::Result;
use std::future::Future;

/// Which index a document belongs to.
///
/// A namespace, the way [`Column`](crate::Column) is a namespace: two
/// indexes can hold keys drawn from the same column — a session's
/// messages and its title both live under `Column::Session` — and a
/// search must not mix them.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextIndex {
    /// One document per session message.
    Messages = 0,
    /// One document per session, over its title and summary.
    SessionMeta = 1,
    /// One document per memory entry.
    Memory = 2,
}

/// A ranked match: the key that was indexed, and how well it scored.
///
/// Bigger is better. A backend whose engine ranks the other way round
/// (sqlite's `bm25()` returns negative-is-better) flips it, so nothing
/// above has to know which convention its store happens to use.
#[derive(Debug, Clone)]
pub struct TextHit {
    pub key: Vec<u8>,
    pub score: f64,
}

/// Ranked full-text search over indexed documents.
pub trait TextSearch: Send + Sync + 'static {
    /// Index `text` under `key`, replacing any document already there.
    ///
    /// `weight` multiplies the document's score at query time. It is a
    /// number and nothing else: what makes one document worth more than
    /// another is the caller's business.
    fn index_text(
        &self,
        index: TextIndex,
        key: &[u8],
        text: &str,
        weight: f64,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Drop the document at `key`. No-op if absent.
    fn drop_text(&self, index: TextIndex, key: &[u8]) -> impl Future<Output = Result<()>> + Send;

    /// Drop every document whose key starts with `prefix` — a session
    /// being deleted takes its messages with it, and that is one call
    /// rather than one per message.
    fn drop_text_prefix(
        &self,
        index: TextIndex,
        prefix: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;

    /// The best `limit` matches, ranked. An empty or unparsable query
    /// returns nothing rather than erroring — a search box is allowed to
    /// contain junk.
    fn search_text(
        &self,
        index: TextIndex,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<TextHit>>> + Send;
}

/// A shared handle is a text index, mirroring the `Arc` impl on
/// [`KVStorage`](crate::KVStorage).
impl<T: TextSearch> TextSearch for std::sync::Arc<T> {
    fn index_text(
        &self,
        index: TextIndex,
        key: &[u8],
        text: &str,
        weight: f64,
    ) -> impl Future<Output = Result<()>> + Send {
        (**self).index_text(index, key, text, weight)
    }

    fn drop_text(&self, index: TextIndex, key: &[u8]) -> impl Future<Output = Result<()>> + Send {
        (**self).drop_text(index, key)
    }

    fn drop_text_prefix(
        &self,
        index: TextIndex,
        prefix: &[u8],
    ) -> impl Future<Output = Result<()>> + Send {
        (**self).drop_text_prefix(index, prefix)
    }

    fn search_text(
        &self,
        index: TextIndex,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<TextHit>>> + Send {
        (**self).search_text(index, query, limit)
    }
}

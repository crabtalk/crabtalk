//! Embedding trait for converting text to vector representations.
//!
//! Used by memory backends that support semantic search (e.g. walrus-sqlite).

use std::future::Future;

/// Converts text into a dense vector embedding.
///
/// Implementations may call external APIs (OpenAI, local models, etc.).
/// Uses RPITIT for async without boxing.
pub trait Embedder: Send + Sync {
    /// Embed the given text into a dense float vector.
    fn embed(&self, text: &str) -> impl Future<Output = Vec<f32>> + Send;
}

/// A no-op embedder that always returns an empty vector.
///
/// Used with [`crate::Memory`] backends that support optional embeddings
/// (e.g. `SqliteMemory<NoEmbedder>`) when no embedding model is configured.
pub struct NoEmbedder;

impl Embedder for NoEmbedder {
    async fn embed(&self, _text: &str) -> Vec<f32> {
        Vec::new()
    }
}

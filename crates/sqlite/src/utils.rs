//! Utility functions for memory recall scoring and ranking.

use agent::MemoryEntry;
use std::collections::HashSet;

/// Cosine similarity between two float vectors.
///
/// Returns 0.0 if either vector is empty or has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b.iter()) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

/// Jaccard similarity between two strings (tokenized by whitespace).
pub(crate) fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

/// Maximal Marginal Relevance re-ranking.
///
/// Selects items that balance relevance (score) against diversity
/// (dissimilarity to already-selected items). Lambda controls the
/// trade-off: 1.0 = pure relevance, 0.0 = pure diversity.
///
/// When `use_cosine` is true and both entries have embeddings, cosine
/// similarity is used for diversity scoring. Falls back to Jaccard
/// (text-based) when embeddings are unavailable.
pub(crate) fn mmr_rerank(
    candidates: Vec<(MemoryEntry, f64)>,
    limit: usize,
    mmr_lambda: f64,
    use_cosine: bool,
) -> Vec<MemoryEntry> {
    let mut remaining: Vec<(MemoryEntry, f64)> = candidates;
    let mut selected: Vec<MemoryEntry> = Vec::with_capacity(limit);

    while selected.len() < limit && !remaining.is_empty() {
        let mut best_idx = 0;
        let mut best_mmr = f64::NEG_INFINITY;

        for (i, (entry, score)) in remaining.iter().enumerate() {
            let max_sim = selected
                .iter()
                .map(|s| {
                    if use_cosine {
                        match (&entry.embedding, &s.embedding) {
                            (Some(a), Some(b)) => cosine_similarity(a, b),
                            _ => jaccard_similarity(&entry.value, &s.value),
                        }
                    } else {
                        jaccard_similarity(&entry.value, &s.value)
                    }
                })
                .fold(0.0_f64, f64::max);
            let mmr_score = mmr_lambda * score - (1.0 - mmr_lambda) * max_sim;
            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_idx = i;
            }
        }

        let (entry, _) = remaining.remove(best_idx);
        selected.push(entry);
    }

    selected
}

/// Decode a little-endian byte blob into a Vec of f32 values.
pub(crate) fn decode_embedding(blob: &[u8]) -> Vec<f32> {
    // Safety: we process in chunks_exact(4) so no out-of-bounds.
    // This avoids per-element array construction overhead.
    let count = blob.len() / 4;
    let mut out = Vec::with_capacity(count);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    out
}

/// Return the current unix timestamp in seconds.
pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

//! Stateless BM25 scorer for memory recall.

use std::collections::HashMap;

const K1: f64 = 1.2;
const B: f64 = 0.75;

/// Tokenize text: lowercase, split on non-alphanumeric, filter stopwords.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && w.len() > 1)
        .map(|w| w.to_lowercase())
        .filter(|w| !is_stopword(w))
        .collect()
}

/// BM25-rank documents against a query. Returns top-`limit` (index, score)
/// pairs sorted by descending score.
pub fn score(docs: &[(usize, &str)], query: &str, limit: usize) -> Vec<(usize, f64)> {
    if docs.is_empty() {
        return Vec::new();
    }

    let mut query_tokens = tokenize(query);
    query_tokens.sort();
    query_tokens.dedup();
    if query_tokens.is_empty() {
        return Vec::new();
    }

    let doc_tokens: Vec<Vec<String>> = docs.iter().map(|(_, text)| tokenize(text)).collect();
    let total_len: usize = doc_tokens.iter().map(|t| t.len()).sum();
    let avgdl = total_len as f64 / docs.len() as f64;

    let n = docs.len() as f64;
    let mut df: HashMap<&str, usize> = HashMap::new();
    for qt in &query_tokens {
        if df.contains_key(qt.as_str()) {
            continue;
        }
        let count = doc_tokens
            .iter()
            .filter(|tokens| tokens.iter().any(|t| t == qt))
            .count();
        df.insert(qt.as_str(), count);
    }

    let mut scores: Vec<(usize, f64)> = docs
        .iter()
        .zip(doc_tokens.iter())
        .map(|((idx, _), tokens)| {
            let dl = tokens.len() as f64;
            let mut doc_score = 0.0;

            let mut tf_map: HashMap<&str, usize> = HashMap::new();
            for t in tokens {
                *tf_map.entry(t.as_str()).or_insert(0) += 1;
            }

            for qt in &query_tokens {
                let doc_freq = *df.get(qt.as_str()).unwrap_or(&0);
                if doc_freq == 0 {
                    continue;
                }

                let idf = ((n - doc_freq as f64 + 0.5) / (doc_freq as f64 + 0.5) + 1.0).ln();
                let tf = *tf_map.get(qt.as_str()).unwrap_or(&0) as f64;
                let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * dl / avgdl));

                doc_score += idf * tf_norm;
            }

            (*idx, doc_score)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(limit);
    scores
}

fn is_stopword(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "is"
            | "it"
            | "in"
            | "of"
            | "to"
            | "and"
            | "or"
            | "for"
            | "on"
            | "at"
            | "by"
            | "with"
            | "as"
            | "be"
            | "was"
            | "are"
            | "been"
            | "has"
            | "had"
            | "have"
            | "do"
            | "does"
            | "did"
            | "but"
            | "not"
            | "no"
            | "if"
            | "so"
            | "from"
            | "that"
            | "this"
            | "then"
            | "than"
            | "into"
            | "its"
            | "my"
            | "me"
            | "we"
            | "he"
            | "she"
            | "they"
            | "you"
            | "your"
            | "our"
            | "his"
            | "her"
    )
}

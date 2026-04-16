//! Inverted-index BM25. Entries register their terms on insert; search
//! walks the posting lists for query terms instead of rescanning every
//! entry on every query.
//!
//! IDF formula is the Lucene variant: `ln((n - df + 0.5)/(df + 0.5) + 1.0)`.
//! The `+1.0` inside the ln keeps IDF non-negative for common terms.

use crate::entry::EntryId;
use std::collections::HashMap;

const K1: f64 = 1.2;
const B: f64 = 0.75;

#[derive(Default)]
pub(crate) struct Index {
    postings: HashMap<String, Vec<(EntryId, u32)>>,
    doc_lens: HashMap<EntryId, u32>,
    total_len: u64,
}

impl Index {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a doc's terms.
    pub(crate) fn insert(&mut self, id: EntryId, terms: &[String]) {
        self.remove(id);
        if terms.is_empty() {
            return;
        }

        let mut tfs: HashMap<&str, u32> = HashMap::new();
        for t in terms {
            *tfs.entry(t.as_str()).or_insert(0) += 1;
        }
        for (term, tf) in tfs {
            self.postings
                .entry(term.to_owned())
                .or_default()
                .push((id, tf));
        }

        let len = terms.len() as u32;
        self.doc_lens.insert(id, len);
        self.total_len += len as u64;
    }

    pub(crate) fn remove(&mut self, id: EntryId) {
        let Some(len) = self.doc_lens.remove(&id) else {
            return;
        };
        self.total_len = self.total_len.saturating_sub(len as u64);
        self.postings.retain(|_, list| {
            list.retain(|(doc, _)| *doc != id);
            !list.is_empty()
        });
    }

    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<(EntryId, f64)> {
        if self.doc_lens.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut query_terms = tokenize(query);
        query_terms.sort();
        query_terms.dedup();
        if query_terms.is_empty() {
            return Vec::new();
        }

        let n = self.doc_lens.len() as f64;
        let avgdl = self.total_len as f64 / n;

        let mut scores: HashMap<EntryId, f64> = HashMap::new();
        for term in &query_terms {
            let Some(postings) = self.postings.get(term) else {
                continue;
            };
            let df = postings.len() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (doc, tf) in postings {
                let dl = *self.doc_lens.get(doc).unwrap_or(&0) as f64;
                let tf = *tf as f64;
                let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * dl / avgdl));
                *scores.entry(*doc).or_insert(0.0) += idf * tf_norm;
            }
        }

        let mut hits: Vec<(EntryId, f64)> = scores.into_iter().filter(|(_, s)| *s > 0.0).collect();
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        hits
    }
}

pub(crate) fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_lowercase)
        .filter(|w| !is_stopword(w))
        .collect()
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

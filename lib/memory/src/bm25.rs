//! Inverted-index BM25. Generic over the document id type so the same
//! implementation backs both memory-entry recall (`EntryId`) and
//! session-message search (`MessageRef` in the runtime crate).
//!
//! IDF formula is the Lucene variant: `ln((n - df + 0.5)/(df + 0.5) + 1.0)`.
//! The `+1.0` inside the ln keeps IDF non-negative for common terms.

use std::{collections::HashMap, hash::Hash};

const K1: f64 = 1.2;
const B: f64 = 0.75;

pub struct Index<Id: Copy + Eq + Hash> {
    postings: HashMap<String, Vec<(Id, u32)>>,
    doc_lens: HashMap<Id, u32>,
    total_len: u64,
}

impl<Id: Copy + Eq + Hash> Default for Index<Id> {
    fn default() -> Self {
        Self {
            postings: HashMap::new(),
            doc_lens: HashMap::new(),
            total_len: 0,
        }
    }
}

impl<Id: Copy + Eq + Hash> Index<Id> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.doc_lens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.doc_lens.is_empty()
    }

    /// Insert or replace a doc's terms.
    pub fn insert(&mut self, id: Id, terms: &[String]) {
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

    pub fn remove(&mut self, id: Id) {
        let Some(len) = self.doc_lens.remove(&id) else {
            return;
        };
        self.total_len = self.total_len.saturating_sub(len as u64);
        self.postings.retain(|_, list| {
            list.retain(|(doc, _)| *doc != id);
            !list.is_empty()
        });
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<(Id, f64)> {
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

        let mut scores: HashMap<Id, f64> = HashMap::new();
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

        let mut hits: Vec<(Id, f64)> = scores.into_iter().filter(|(_, s)| *s > 0.0).collect();
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        hits
    }
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_lowercase)
        .filter(|w| !is_stopword(w))
        .collect()
}

pub fn is_stopword(word: &str) -> bool {
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

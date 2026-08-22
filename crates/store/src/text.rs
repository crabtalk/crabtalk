//! Ranked full-text search — the one thing keys cannot answer.
//!
//! [`TextSearch`] is an interface, and BM25 over [`KVStorage`] is one
//! implementation of it: an inverted index is a map from term to the
//! documents containing it, and a map is what a KV store is. Any store
//! that can get, put and scan gets it free. A backend whose engine
//! already ranks text implements the four methods over that instead.
//!
//! ```text
//! idx/text/{ix}/doc/{key}          len, weight, and the doc's terms
//! idx/text/{ix}/term/{term}/{key}  term frequency in that doc
//! idx/text/{ix}/stats              document count and total length
//! ```
//!
//! The doc record is what makes removal cheap: it names the terms to
//! retract, so dropping a document touches its own postings rather than
//! walking the index.

use crate::kv::{Column, KVStorage};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, future::Future};

/// Which index a document belongs to.
///
/// A namespace, the way [`Column`] is a namespace: two indexes can hold
/// keys drawn from the same column — a session's messages and its title
/// both live under `Column::Session` — and a search must not mix them.
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

impl TextIndex {
    fn as_str(&self) -> &'static str {
        match self {
            TextIndex::Messages => "msg",
            TextIndex::SessionMeta => "meta",
            TextIndex::Memory => "mem",
        }
    }
}

/// A ranked match: the key that was indexed, and how well it scored.
/// Bigger is better.
#[derive(Debug, Clone)]
pub struct TextHit {
    pub key: Vec<u8>,
    pub score: f64,
}

/// BM25's two knobs, for the implementation over [`KVStorage`].
///
/// `k1` is how fast a repeated term stops helping; `b` is how hard a
/// long document is penalised for its length. The defaults are the
/// values BM25 is usually published with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bm25 {
    pub k1: f64,
    pub b: f64,
}

impl Default for Bm25 {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// What is known about one indexed document.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Doc {
    pub len: u32,
    pub weight: f64,
    pub terms: Vec<String>,
}

/// How many documents there are, and how long they are in total — the
/// two numbers BM25 needs that no single document knows.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Stats {
    pub docs: u64,
    pub total_len: u64,
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

    /// The best `limit` matches, ranked.
    ///
    /// A term ending in `*` matches every term it prefixes. An empty or
    /// all-stopword query returns nothing rather than erroring — a
    /// search box is allowed to contain junk.
    fn search_text(
        &self,
        index: TextIndex,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<TextHit>>> + Send;
}

/// BM25 over the keyspace, for any store that can get, put and scan.
impl<T: KVStorage> TextSearch for T {
    async fn index_text(
        &self,
        index: TextIndex,
        key: &[u8],
        text: &str,
        weight: f64,
    ) -> Result<()> {
        self.drop_text(index, key).await?;
        let terms = tokenize(text);
        if terms.is_empty() {
            return Ok(());
        }

        let mut tfs: HashMap<&str, u32> = HashMap::new();
        for term in &terms {
            *tfs.entry(term.as_str()).or_insert(0) += 1;
        }
        for (term, tf) in &tfs {
            self.put(
                Column::Text,
                &self.posting_key(index, term, key),
                tf.to_string().as_bytes(),
            )
            .await?;
        }

        let len = terms.len() as u32;
        let mut unique: Vec<String> = tfs.keys().map(|t| (*t).to_owned()).collect();
        unique.sort();
        self.put_json(
            Column::Text,
            &self.doc_key(index, key),
            &Doc {
                len,
                weight,
                terms: unique,
            },
        )
        .await?;

        let mut stats = self.stats(index).await?;
        stats.docs += 1;
        stats.total_len += len as u64;
        self.put_json(Column::Text, &self.stats_key(index), &stats)
            .await
    }

    async fn drop_text(&self, index: TextIndex, key: &[u8]) -> Result<()> {
        let doc_key = self.doc_key(index, key);
        let Some(doc) = self.get_json::<Doc>(Column::Text, &doc_key).await? else {
            return Ok(());
        };
        // The doc record names its own postings, so retracting one
        // document never walks the index.
        for term in &doc.terms {
            self.delete(Column::Text, &self.posting_key(index, term, key))
                .await?;
        }
        self.delete(Column::Text, &doc_key).await?;

        let mut stats = self.stats(index).await?;
        stats.docs = stats.docs.saturating_sub(1);
        stats.total_len = stats.total_len.saturating_sub(doc.len as u64);
        self.put_json(Column::Text, &self.stats_key(index), &stats)
            .await
    }

    async fn drop_text_prefix(&self, index: TextIndex, prefix: &[u8]) -> Result<()> {
        let scope = self.doc_key(index, prefix);
        for doc_key in self.scan_keys(Column::Text, &scope).await? {
            // Recover the document key the doc record was filed under.
            let Some(key) = doc_key.get(self.doc_prefix(index).len()..) else {
                continue;
            };
            self.drop_text(index, key).await?;
        }
        Ok(())
    }

    async fn search_text(
        &self,
        index: TextIndex,
        query: &str,
        limit: usize,
    ) -> Result<Vec<TextHit>> {
        let terms = query_terms(query);
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let stats = self.stats(index).await?;
        if stats.docs == 0 {
            return Ok(Vec::new());
        }
        let n = stats.docs as f64;
        let avgdl = stats.total_len as f64 / n;
        let Bm25 { k1, b } = Bm25::default();

        // term → postings, then postings → scores. Documents are
        // read once each, after the terms have named them all.
        let mut scores: HashMap<Vec<u8>, f64> = HashMap::new();
        let mut lens: HashMap<Vec<u8>, (f64, f64)> = HashMap::new();
        for term in &terms {
            let postings = self.postings(index, term).await?;
            if postings.is_empty() {
                continue;
            }
            // A prefix's document frequency is the union it matches,
            // which is what makes a broad prefix weigh less.
            let df = postings.len() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (key, tf) in postings {
                let (dl, weight) = match lens.get(&key) {
                    Some(known) => *known,
                    None => {
                        let doc = self
                            .get_json::<Doc>(Column::Text, &self.doc_key(index, &key))
                            .await?
                            .unwrap_or_default();
                        let known = (doc.len as f64, doc.weight);
                        lens.insert(key.clone(), known);
                        known
                    }
                };
                let tf = tf as f64;
                let norm = (tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * dl / avgdl));
                *scores.entry(key).or_insert(0.0) += idf * norm * weight;
            }
        }

        let mut hits: Vec<TextHit> = scores
            .into_iter()
            .filter(|(_, score)| *score > 0.0)
            .map(|(key, score)| TextHit { key, score })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.key.cmp(&b.key)));
        hits.truncate(limit);
        Ok(hits)
    }
}

/// The keyspace BM25 is written over. Private: a store that ranks text
/// its own way has none of these.
trait TextKv: KVStorage {
    fn doc_prefix(&self, index: TextIndex) -> Vec<u8> {
        self.prefix(&["idx", "text", index.as_str(), "doc"])
    }

    fn doc_key(&self, index: TextIndex, key: &[u8]) -> Vec<u8> {
        let mut out = self.doc_prefix(index);
        out.extend_from_slice(key);
        out
    }

    fn posting_key(&self, index: TextIndex, term: &str, key: &[u8]) -> Vec<u8> {
        let mut out = self.prefix(&["idx", "text", index.as_str(), "term", term]);
        out.extend_from_slice(key);
        out
    }

    fn stats_key(&self, index: TextIndex) -> Vec<u8> {
        self.key(&["idx", "text", index.as_str(), "stats"])
    }

    fn stats(&self, index: TextIndex) -> impl Future<Output = Result<Stats>> + Send {
        async move {
            Ok(self
                .get_json(Column::Text, &self.stats_key(index))
                .await?
                .unwrap_or_default())
        }
    }

    /// Every `(document, term frequency)` for one term, or for every
    /// term a `foo*` prefixes.
    fn postings(
        &self,
        index: TextIndex,
        term: &Term,
    ) -> impl Future<Output = Result<Vec<(Vec<u8>, u32)>>> + Send {
        async move {
            let scope = match term.prefix {
                // No trailing separator: `deploy` must also reach
                // `deployment`, so the scan stops at the term itself.
                true => self.key(&["idx", "text", index.as_str(), "term", &term.text]),
                false => self.prefix(&["idx", "text", index.as_str(), "term", &term.text]),
            };
            let rows = self.scan(Column::Text, &scope).await?;
            let mut out = Vec::with_capacity(rows.len());
            for (key, tf) in rows {
                // The document key is whatever follows `…/term/{term}/`.
                let Some(cut) = find_term_boundary(&key, &scope, term.prefix) else {
                    continue;
                };
                let Ok(tf) = std::str::from_utf8(&tf).unwrap_or("0").parse::<u32>() else {
                    continue;
                };
                out.push((key[cut..].to_vec(), tf));
            }
            Ok(out)
        }
    }
}

impl<T: KVStorage + ?Sized> TextKv for T {}

/// One term of a query, and whether it was written `foo*`.
pub struct Term {
    text: String,
    prefix: bool,
}

/// Split a query into terms, honouring a trailing `*`.
fn query_terms(query: &str) -> Vec<Term> {
    let mut out: Vec<Term> = Vec::new();
    for raw in query.split_whitespace() {
        let prefix = raw.ends_with('*');
        let stripped = raw.trim_end_matches('*');
        // A prefix is matched against terms rather than being one, so it
        // skips the stopword filter that would drop `an*` or `be*`.
        let text = match prefix {
            true => normalize(stripped),
            false => tokenize(stripped).into_iter().next().unwrap_or_default(),
        };
        if text.is_empty() {
            continue;
        }
        if !out.iter().any(|t| t.text == text && t.prefix == prefix) {
            out.push(Term { text, prefix });
        }
    }
    out
}

fn normalize(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Where the document key starts, given the scope that was scanned.
///
/// An exact-term scan is a clean prefix. A prefix scan is not: the key
/// holds `…/term/{matched}/{doc}` where `{matched}` is longer than what
/// was asked for, so the separator after it has to be found.
fn find_term_boundary(key: &[u8], scope: &[u8], prefix: bool) -> Option<usize> {
    if !prefix {
        return Some(scope.len());
    }
    key.get(scope.len()..)?
        .iter()
        .position(|b| *b == b'/')
        .map(|at| scope.len() + at + 1)
}

/// Split text into searchable terms: lowercased, alphanumeric, longer
/// than one character, and not a stopword.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_lowercase)
        .filter(|w| !is_stopword(w))
        .collect()
}

/// Words too common to discriminate between documents.
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

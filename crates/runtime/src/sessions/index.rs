//! BM25 session index, keyed by `(session_id, msg_idx)`. One doc per
//! indexed message. Per-role weighting is applied at search time to
//! the raw BM25 scores; the underlying inverted index stays kind-blind.
//!
//! `session_id` is an internal counter — agents and external callers
//! identify sessions by their `SessionHandle` (the storage slug). The
//! index maintains the `handle → session_id` mapping so live appends
//! and cold-start rebuilds resolve to the same internal id.

use super::hit::{
    MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHit, WindowItem,
};
use memory::bm25;
use std::collections::HashMap;
use wcore::model::{HistoryEntry, Role};
use wcore::storage::SessionHandle;

/// Document identity in the session index — one doc per message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageRef {
    pub session_id: u64,
    pub msg_idx: u32,
}

/// Per-message metadata cached alongside the BM25 index. Holds the
/// snippet + role we need to rebuild a `WindowItem` without going
/// back to disk on every search hit.
#[derive(Debug, Clone)]
struct DocMeta {
    role: Role,
    snippet: String,
    truncated: bool,
    tool_name: Option<String>,
}

/// Per-session metadata — handle and basic identity, used to populate
/// `SessionHit` without a storage lookup.
#[derive(Debug, Clone)]
struct SessionMeta {
    handle: SessionHandle,
    agent: String,
    sender: String,
    title: String,
    created_at: String,
    updated_at: String,
}

/// Per-field boost factor applied to the title's BM25 contribution
/// when scoring a session hit. Defaults from RFC 0185.
const TITLE_BOOST: f64 = 2.0;

/// Per-field boost factor applied to the summary's BM25 contribution.
const SUMMARY_BOOST: f64 = 3.0;

/// Multiplier on `opts.limit` for raw hits we pull from the message
/// index before role-weighting and filters trim the result. Bigger
/// values help recall after filtering at the cost of more sort work;
/// 4× is a balance that holds up at the current `MAX_HITS_PER_QUERY`.
const RAW_OVERSHOOT: usize = 4;

pub struct SessionIndex {
    bm25: bm25::Index<MessageRef>,
    docs: HashMap<MessageRef, DocMeta>,
    by_session: HashMap<u64, Vec<u32>>,
    sessions: HashMap<u64, SessionMeta>,
    handle_to_id: HashMap<String, u64>,
    next_id: u64,
    /// Per-session title BM25, doc-keyed by session_id.
    title_index: bm25::Index<u64>,
    /// Per-session summary BM25, doc-keyed by session_id.
    summary_index: bm25::Index<u64>,
}

impl Default for SessionIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionIndex {
    pub fn new() -> Self {
        Self {
            bm25: bm25::Index::new(),
            docs: HashMap::new(),
            by_session: HashMap::new(),
            sessions: HashMap::new(),
            handle_to_id: HashMap::new(),
            next_id: 1,
            title_index: bm25::Index::new(),
            summary_index: bm25::Index::new(),
        }
    }

    pub fn message_count(&self) -> usize {
        self.docs.len()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Look up the internal `session_id` for a storage handle. Returns
    /// `None` if the session has never been registered. The id is an
    /// index-internal counter — callers outside this module should not
    /// rely on its stability across process restarts.
    pub fn handle_to_session_id(&self, handle: &str) -> Option<u64> {
        self.handle_to_id.get(handle).copied()
    }

    /// Register or look up a session by its storage handle. Idempotent
    /// — repeat calls return the same `session_id`. Mutable fields
    /// (`title`, `updated_at`, `summary`) are refreshed on every call
    /// so the index stays in step with the meta line.
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_session(
        &mut self,
        handle: &SessionHandle,
        agent: &str,
        sender: &str,
        title: &str,
        summary: Option<&str>,
        created_at: &str,
        updated_at: &str,
    ) -> u64 {
        if let Some(&id) = self.handle_to_id.get(handle.as_str()) {
            if let Some(meta) = self.sessions.get_mut(&id) {
                if !title.is_empty() {
                    meta.title = title.to_owned();
                }
                if !updated_at.is_empty() {
                    meta.updated_at = updated_at.to_owned();
                }
            }
            self.reindex_title(id, title);
            self.reindex_summary(id, summary);
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.handle_to_id.insert(handle.as_str().to_owned(), id);
        self.sessions.insert(
            id,
            SessionMeta {
                handle: handle.clone(),
                agent: agent.to_owned(),
                sender: sender.to_owned(),
                title: title.to_owned(),
                created_at: created_at.to_owned(),
                updated_at: updated_at.to_owned(),
            },
        );
        self.reindex_title(id, title);
        self.reindex_summary(id, summary);
        id
    }

    fn reindex_title(&mut self, id: u64, title: &str) {
        if title.is_empty() {
            self.title_index.remove(id);
            return;
        }
        let terms = bm25::tokenize(title);
        self.title_index.insert(id, &terms);
    }

    fn reindex_summary(&mut self, id: u64, summary: Option<&str>) {
        match summary.filter(|s| !s.is_empty()) {
            Some(s) => {
                let terms = bm25::tokenize(s);
                self.summary_index.insert(id, &terms);
            }
            None => {
                self.summary_index.remove(id);
            }
        }
    }

    /// Drop a session from the index. Used when a session is deleted
    /// or before re-indexing after compaction.
    pub fn forget_session(&mut self, session_id: u64) {
        if let Some(idxs) = self.by_session.remove(&session_id) {
            for msg_idx in idxs {
                let key = MessageRef {
                    session_id,
                    msg_idx,
                };
                self.bm25.remove(key);
                self.docs.remove(&key);
            }
        }
        if let Some(meta) = self.sessions.remove(&session_id) {
            self.handle_to_id.remove(meta.handle.as_str());
        }
        self.title_index.remove(session_id);
        self.summary_index.remove(session_id);
    }

    /// Index one message. Auto-injected entries are skipped — they
    /// never reach storage so they shouldn't reach the index. Returns
    /// the assigned `msg_idx` (the message's position within this
    /// session's indexed log).
    ///
    /// Tool-result and system messages are *not* fed to the BM25
    /// posting list — tool outputs frequently carry credentials, file
    /// contents, or other secrets that shouldn't be findable via
    /// free-text search. They still get a `DocMeta` entry so they
    /// appear in window context for hits anchored on adjacent
    /// messages — once a caller has a session, the conversation
    /// transcript was already accessible to them.
    pub fn insert_message(&mut self, session_id: u64, entry: &HistoryEntry) -> Option<u32> {
        if entry.auto_injected {
            return None;
        }
        let msg_idx = self.by_session.entry(session_id).or_default().len() as u32;
        let key = MessageRef {
            session_id,
            msg_idx,
        };
        let indexable = extract_indexable_text(entry);
        if !indexable.is_empty() {
            let terms = bm25::tokenize(&indexable);
            self.bm25.insert(key, &terms);
        }
        let (snippet, truncated) = make_snippet(entry);
        self.docs.insert(
            key,
            DocMeta {
                role: entry.role().clone(),
                snippet,
                truncated,
                tool_name: extract_tool_name(entry),
            },
        );
        self.by_session
            .get_mut(&session_id)
            .expect("by_session entry created above")
            .push(msg_idx);
        Some(msg_idx)
    }

    /// Run a search and shape hits with windowed context. Limits clamp
    /// to `MAX_HITS_PER_QUERY` and `MAX_WINDOW_ITEMS`. Per-session
    /// title and summary contributions are added to the best message
    /// hit's score with their respective boost factors.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> Vec<SessionHit> {
        let limit = opts.limit.clamp(1, MAX_HITS_PER_QUERY);
        let context_before = opts.context_before;
        let context_after = opts.context_after;

        // Pull more raw hits than we'll keep — role-weighting and
        // session-level boosts can reshuffle top-K, and filters drop
        // arbitrary candidates.
        let raw_limit = (limit * RAW_OVERSHOOT).clamp(limit, MAX_HITS_PER_QUERY * RAW_OVERSHOOT);
        let raw = self.bm25.search(query, raw_limit);

        // Pre-compute session-level boosts for any session that has
        // a non-zero title or summary contribution to this query.
        // Empty indexes short-circuit cheaply inside `bm25::search`.
        let title_boost: HashMap<u64, f64> = self
            .title_index
            .search(query, MAX_HITS_PER_QUERY * RAW_OVERSHOOT)
            .into_iter()
            .map(|(id, score)| (id, score * TITLE_BOOST))
            .collect();
        let summary_boost: HashMap<u64, f64> = self
            .summary_index
            .search(query, MAX_HITS_PER_QUERY * RAW_OVERSHOOT)
            .into_iter()
            .map(|(id, score)| (id, score * SUMMARY_BOOST))
            .collect();

        let mut hits: Vec<SessionHit> = Vec::with_capacity(raw.len());
        let mut seen: HashMap<u64, ()> = HashMap::new();
        for (mref, score) in raw {
            let Some(session) = self.sessions.get(&mref.session_id) else {
                continue;
            };
            if let Some(ref a) = opts.agent_filter
                && session.agent != *a
            {
                continue;
            }
            if let Some(ref s) = opts.sender_filter
                && session.sender != *s
            {
                continue;
            }
            // One hit per session: results are deduplicated by session
            // so a long thread doesn't flood the response. Callers
            // wanting more depth into a single session widen the
            // window or follow up with `list_messages`.
            if seen.insert(mref.session_id, ()).is_some() {
                continue;
            }
            let Some(meta) = self.docs.get(&mref) else {
                continue;
            };
            let mut weighted = score * role_weight(&meta.role, meta.tool_name.is_some());
            weighted += title_boost.get(&mref.session_id).copied().unwrap_or(0.0);
            weighted += summary_boost.get(&mref.session_id).copied().unwrap_or(0.0);
            let window = self.build_window(mref, context_before, context_after);
            hits.push(SessionHit {
                session_handle: session.handle.clone(),
                msg_idx: mref.msg_idx,
                score: weighted,
                title: session.title.clone(),
                agent: session.agent.clone(),
                sender: session.sender.clone(),
                created_at: session.created_at.clone(),
                updated_at: session.updated_at.clone(),
                window,
            });
            if hits.len() >= limit {
                break;
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits
    }

    fn build_window(&self, anchor: MessageRef, before: usize, after: usize) -> Vec<WindowItem> {
        let Some(idxs) = self.by_session.get(&anchor.session_id) else {
            return Vec::new();
        };
        // `idxs` is append-only and monotonically increasing in
        // msg_idx, so binary_search is correct and keeps build_window
        // O(log n) instead of O(n) for sessions with many messages.
        let pos = idxs
            .binary_search(&anchor.msg_idx)
            .unwrap_or_else(|insert_pos| insert_pos.saturating_sub(1));
        let start = pos.saturating_sub(before);
        let end = (pos + after + 1).min(idxs.len());
        let mut span: Vec<u32> = idxs[start..end].to_vec();
        if span.len() > MAX_WINDOW_ITEMS {
            // Drop from the edges, keeping the anchor centred.
            let overflow = span.len() - MAX_WINDOW_ITEMS;
            let drop_before = overflow / 2;
            let drop_after = overflow - drop_before;
            span = span[drop_before..span.len() - drop_after].to_vec();
        }
        span.into_iter()
            .filter_map(|msg_idx| {
                let key = MessageRef {
                    session_id: anchor.session_id,
                    msg_idx,
                };
                let meta = self.docs.get(&key)?;
                Some(WindowItem {
                    role: meta.role.clone(),
                    msg_idx,
                    snippet: meta.snippet.clone(),
                    truncated: meta.truncated,
                    tool_name: meta.tool_name.clone(),
                })
            })
            .collect()
    }
}

/// Text fed to BM25 for indexing. User and assistant messages
/// contribute their content; tool-call assistants contribute only the
/// function names (arguments may carry secrets); tool-result and
/// system messages contribute nothing — they're skipped to keep
/// credentials and other sensitive output out of free-text search.
fn extract_indexable_text(entry: &HistoryEntry) -> String {
    match entry.role() {
        Role::User | Role::Assistant => {
            let text = entry.text();
            if !text.is_empty() {
                return text.to_owned();
            }
            // Tool-call assistants have no text — index function names
            // so "find sessions where I ran shell" works. Arguments
            // are deliberately excluded.
            entry
                .tool_calls()
                .iter()
                .map(|tc| tc.function.name.clone())
                .collect::<Vec<_>>()
                .join(" ")
        }
        _ => String::new(),
    }
}

/// Display text for window items. Unlike indexable text, this returns
/// the actual content of every role — once a caller has a hit, the
/// surrounding transcript is part of the context they need to read
/// the conversation.
fn extract_display_text(entry: &HistoryEntry) -> String {
    let text = entry.text();
    if !text.is_empty() {
        return text.to_owned();
    }
    if !entry.tool_calls().is_empty() {
        return entry
            .tool_calls()
            .iter()
            .map(|tc| format!("{}({})", tc.function.name, tc.function.arguments))
            .collect::<Vec<_>>()
            .join(" ");
    }
    String::new()
}

fn make_snippet(entry: &HistoryEntry) -> (String, bool) {
    let raw = extract_display_text(entry);
    if raw.len() <= MAX_SNIPPET_BYTES {
        return (raw, false);
    }
    let mut end = MAX_SNIPPET_BYTES;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    (raw[..end].to_owned(), true)
}

fn extract_tool_name(entry: &HistoryEntry) -> Option<String> {
    if matches!(entry.role(), Role::Tool) {
        return entry.message.name.clone().filter(|s| !s.is_empty());
    }
    let calls = entry.tool_calls();
    if calls.is_empty() {
        return None;
    }
    Some(calls[0].function.name.clone())
}

/// Per-role / per-kind score multiplier. Defaults from the community
/// Claude Code conversation-search pattern (alexop.dev,
/// `raine/claude-history`); see RFC 0185 for the calibration plan.
fn role_weight(role: &Role, has_tool_call: bool) -> f64 {
    match role {
        Role::User => 1.5,
        Role::Assistant if has_tool_call => 1.3,
        Role::Assistant => 1.0,
        Role::Tool => 1.3,
        Role::System => 0.5,
        _ => 1.0,
    }
}

//! Persisted sessions, their message streams, and search over them.

use crate::{
    AgentConfig, AgentId, HistoryEntry,
    kv::{Column, KVStorage},
    session::{
        EventLine, MAX_HITS_PER_QUERY, MAX_WINDOW_ITEMS, SearchOptions, SessionHandle, SessionHit,
        SessionMeta, SessionSnapshot, WindowItem,
    },
    text::{TextIndex, TextSearch},
};
use anyhow::Result;
use std::{collections::BTreeMap, future::Future};

/// How a store ranks a session search.
///
/// Every number here is a judgement about what relevance means, and none
/// of them is derivable from the data — which is why they are asked for
/// rather than fixed. [`Default`] is a starting point, not an answer:
/// a store whose sessions are mostly tool traffic wants different
/// numbers from one that is mostly conversation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// What a person's own words are worth. They say what a session was
    /// about better than anything else in it.
    pub user: f64,
    /// What a tool call's name is worth — a real signal, but a weaker
    /// one. Its arguments are never indexed at all.
    pub tool_call: f64,
    /// Added when the query also matches the session's title.
    pub title_boost: f64,
    /// Added when it matches the summary. A summary is written about the
    /// whole session, so it says more than a title someone typed once.
    pub summary_boost: f64,
    /// Message matches to pull per requested hit.
    ///
    /// The index ranks messages; a result is a session — its single best
    /// message. One session mentioning a term repeatedly would otherwise
    /// fill a flat top-N by itself, so over-fetch and group. A heuristic
    /// bound, not a guarantee: when the candidate set comes back full,
    /// that is logged rather than passed off as a complete answer.
    pub candidates_per_hit: usize,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            user: 1.5,
            tool_call: 1.3,
            title_boost: 2.0,
            summary_boost: 3.0,
            candidates_per_hit: 10,
        }
    }
}

/// Persisted sessions and their message streams.
pub trait Sessions: KVStorage + TextSearch {
    /// How this store ranks.
    ///
    /// The numbers live here rather than in constants because they are
    /// judgements, and a judgement should have a name and a place to be
    /// changed. One place, for now: this trait is blanket-implemented,
    /// so the default is what every store gets.
    fn config(&self) -> Weights {
        Weights::default()
    }

    /// Persist a brand-new session under `handle` — chosen by the
    /// caller, not minted here. The handle encodes nothing, so renaming
    /// an agent never orphans its transcripts.
    fn create_session(
        &self,
        handle: &SessionHandle,
        agent: &AgentId,
        created_by: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let now = chrono::Utc::now().to_rfc3339();
            let meta = SessionMeta {
                agent: *agent,
                created_by: created_by.to_owned(),
                created_at: now.clone(),
                title: String::new(),
                updated_at: now,
                message_count: 0,
                summary: None,
            };
            self.put_json(Column::Session, &self.meta_key(handle), &meta)
                .await?;
            self.put(
                Column::Session,
                &self.session_index_key(&meta, handle),
                handle.as_str().as_bytes(),
            )
            .await?;
            Ok(())
        }
    }

    fn load_session(
        &self,
        handle: &SessionHandle,
    ) -> impl Future<Output = Result<Option<SessionSnapshot>>> + Send {
        async move {
            let Some(meta) = self.session_meta(handle).await? else {
                return Ok(None);
            };
            let archive: Option<String> = self
                .get_json(Column::Session, &self.archive_key(handle))
                .await?;

            let keys = self
                .scan_keys(Column::Session, &self.message_prefix(handle))
                .await?;
            let mut history = Vec::with_capacity(keys.len());
            for key in keys {
                match self.get_json::<HistoryEntry>(Column::Session, &key).await {
                    Ok(Some(entry)) => history.push(entry),
                    Ok(None) => {}
                    Err(e) => tracing::warn!("skipping unreadable session message: {e}"),
                }
            }
            Ok(Some(SessionSnapshot {
                meta,
                history,
                archive,
            }))
        }
    }

    fn list_sessions(
        &self,
    ) -> impl Future<Output = Result<Vec<(SessionHandle, SessionMeta)>>> + Send {
        async move {
            let mut out = Vec::new();
            for handle in self.indexed_handles(None).await? {
                if let Some(meta) = self.session_meta(&handle).await? {
                    out.push((handle, meta));
                }
            }
            // Recency ordering is a property of the answer, not of the
            // keys: `updated_at` moves on every append, and an index
            // keyed by it would have to be rewritten each time.
            out.sort_by(|(ah, am), (bh, bm)| {
                bm.updated_at
                    .cmp(&am.updated_at)
                    .then_with(|| bm.created_at.cmp(&am.created_at))
                    .then_with(|| ah.as_str().cmp(bh.as_str()))
            });
            Ok(out)
        }
    }

    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            if entries.is_empty() {
                return Ok(());
            }
            let Some(mut meta) = self.session_meta(handle).await? else {
                anyhow::bail!("session not found: {}", handle.as_str());
            };
            let weights = self.config();
            let from = meta.message_count as usize;
            for (offset, entry) in entries.iter().enumerate() {
                let key = self.message_key(handle, from + offset);
                self.put_json(Column::Session, &key, entry).await?;
                // `indexable` decides what may be searched at all — tool
                // results and tool-call arguments are excluded there, so
                // the index never sees a credential that passed through
                // a message.
                if let Some((body, role)) = entry.indexable() {
                    let weight = match role {
                        "user" => weights.user,
                        "assistant_tool" => weights.tool_call,
                        _ => 1.0,
                    };
                    self.index_text(TextIndex::Messages, &key, &body, weight)
                        .await?;
                }
            }
            meta.message_count += entries.len() as u64;
            meta.updated_at = chrono::Utc::now().to_rfc3339();
            self.put_json(Column::Session, &self.meta_key(handle), &meta)
                .await
        }
    }

    fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let prefix = self.prefix(&["session", handle.as_str(), "evt"]);
            let mut idx = self.scan_keys(Column::Session, &prefix).await?.len();
            for event in events {
                let key = self.key(&["session", handle.as_str(), "evt", &format!("{idx:012}")]);
                self.put_json(Column::Session, &key, event).await?;
                idx += 1;
            }
            Ok(())
        }
    }

    /// Record a compaction boundary. `archive` names the memory entry
    /// holding the summary — the marker carries the pointer, never the
    /// text.
    fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            // The compacted prefix leaves the live history: the marker is
            // a pointer to where the text went, so the messages it covers
            // are dropped rather than kept beside it.
            self.truncate_session_messages(handle, 0).await?;
            self.put_json(Column::Session, &self.archive_key(handle), &archive)
                .await
        }
    }

    fn truncate_session_messages(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let Some(mut meta) = self.session_meta(handle).await? else {
                return Ok(());
            };
            let keys = self
                .scan_keys(Column::Session, &self.message_prefix(handle))
                .await?;
            for key in keys.iter().skip(keep) {
                self.drop_text(TextIndex::Messages, key).await?;
                self.delete(Column::Session, key).await?;
            }
            meta.message_count = keep.min(keys.len()) as u64;
            self.put_json(Column::Session, &self.meta_key(handle), &meta)
                .await
        }
    }

    /// Update the mutable half of a session's metadata: title, summary,
    /// timestamps.
    ///
    /// `message_count` is not the caller's to set. It counts message
    /// keys, `append` and `truncate` maintain it, and a caller sending
    /// its own would be sending a live history length — which counts the
    /// archive prefix and per-run framing, neither of which is a message
    /// key. So the stored value wins over whatever arrives here.
    fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &SessionMeta,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let mut meta = meta.clone();
            if let Some(stored) = self.session_meta(handle).await? {
                meta.message_count = stored.message_count;
            }
            self.put_json(Column::Session, &self.meta_key(handle), &meta)
                .await?;
            self.index_meta_text(handle, &meta).await
        }
    }

    fn delete_session(&self, handle: &SessionHandle) -> impl Future<Output = Result<bool>> + Send {
        async move {
            // Index first. A crash here leaves content nothing can reach,
            // which a sweep collects; the reverse leaves an entry
            // pointing at a transcript that is not there.
            let prefix = self.prefix(&["session", handle.as_str()]);
            self.drop_text_prefix(TextIndex::Messages, &prefix).await?;
            self.drop_text_prefix(TextIndex::SessionMeta, &prefix)
                .await?;
            if let Some(meta) = self.session_meta(handle).await? {
                self.delete(Column::Session, &self.session_index_key(&meta, handle))
                    .await?;
            }
            let mut existed = false;
            for key in self.scan_keys(Column::Session, &prefix).await? {
                existed |= self.delete(Column::Session, &key).await?;
            }
            Ok(existed)
        }
    }

    fn delete_sessions_of(&self, agent: &AgentId) -> impl Future<Output = Result<usize>> + Send {
        async move {
            let mut purged = 0;
            for handle in self.indexed_handles(Some(agent)).await? {
                if self.delete_session(&handle).await? {
                    purged += 1;
                }
            }
            Ok(purged)
        }
    }

    /// Rank messages, keep each session's best, then boost by what the
    /// session as a whole says about itself.
    ///
    /// Three steps because they read different stores: the text index
    /// knows which message matched, KV holds the metadata a hit is
    /// filtered and rendered by, and the window is more KV.
    fn search_sessions(
        &self,
        query: &str,
        opts: &SearchOptions,
    ) -> impl Future<Output = Result<Vec<SessionHit>>> + Send {
        async move {
            let limit = opts.limit.clamp(1, MAX_HITS_PER_QUERY);
            let candidates = limit.saturating_mul(self.config().candidates_per_hit);
            let matches = self
                .search_text(TextIndex::Messages, query, candidates)
                .await?;
            if matches.len() == candidates {
                tracing::debug!(
                    "session search hit the candidate ceiling ({candidates}); \
                     some sessions may be missing from the results"
                );
            }

            // Best message per session.
            let mut best: BTreeMap<String, (usize, f64)> = BTreeMap::new();
            for hit in &matches {
                let Some((handle, idx)) = parse_message_key(&hit.key) else {
                    continue;
                };
                best.entry(handle)
                    .and_modify(|entry| {
                        if hit.score > entry.1 {
                            *entry = (idx, hit.score);
                        }
                    })
                    .or_insert((idx, hit.score));
            }
            if best.is_empty() {
                return Ok(Vec::new());
            }

            let boosts = self.meta_boosts(query, candidates).await?;

            let mut hits = Vec::new();
            for (handle, (idx, score)) in best {
                let handle = SessionHandle::new(handle);
                let Some(meta) = self.session_meta(&handle).await? else {
                    continue;
                };
                if opts.agent_filter.is_some_and(|id| meta.agent != id) {
                    continue;
                }
                if opts
                    .sender_filter
                    .as_ref()
                    .is_some_and(|s| &meta.created_by != s)
                {
                    continue;
                }
                // A hit is read by a person or a model, and a bare ULID
                // names nothing.
                let agent_name = self
                    .get_json::<AgentConfig>(
                        Column::Agent,
                        &self.key(&["agent", &meta.agent.to_string()]),
                    )
                    .await
                    .ok()
                    .flatten()
                    .map(|c| c.name)
                    .unwrap_or_default();
                hits.push(SessionHit {
                    msg_idx: idx as u32,
                    score: score + boosts.get(handle.as_str()).copied().unwrap_or(0.0),
                    title: meta.title.clone(),
                    agent: meta.agent,
                    agent_name,
                    sender: meta.created_by.clone(),
                    created_at: meta.created_at.clone(),
                    updated_at: meta.updated_at.clone(),
                    window: self.window(&handle, idx, opts).await?,
                    session_handle: handle,
                });
            }
            hits.sort_by(|a, b| b.score.total_cmp(&a.score));
            hits.truncate(limit);
            Ok(hits)
        }
    }

    // ── Keys and helpers ───────────────────────────────────────────

    fn meta_key(&self, handle: &SessionHandle) -> Vec<u8> {
        self.key(&["session", handle.as_str(), "meta"])
    }

    fn archive_key(&self, handle: &SessionHandle) -> Vec<u8> {
        self.key(&["session", handle.as_str(), "archive"])
    }

    /// Zero-padded so a prefix scan returns messages in order: keys sort
    /// as bytes, and "10" sorts before "2".
    fn message_key(&self, handle: &SessionHandle, idx: usize) -> Vec<u8> {
        self.key(&["session", handle.as_str(), "msg", &format!("{idx:012}")])
    }

    fn message_prefix(&self, handle: &SessionHandle) -> Vec<u8> {
        self.prefix(&["session", handle.as_str(), "msg"])
    }

    /// `(agent, created_by, created_at)` → handle. Ordered by
    /// construction, which is what makes `indexed_handles` a scan.
    fn session_index_key(&self, meta: &SessionMeta, handle: &SessionHandle) -> Vec<u8> {
        self.key(&[
            "idx",
            "sess",
            &meta.agent.to_string(),
            &meta.created_by,
            &meta.created_at,
            handle.as_str(),
        ])
    }

    fn session_meta(
        &self,
        handle: &SessionHandle,
    ) -> impl Future<Output = Result<Option<SessionMeta>>> + Send {
        async move { self.get_json(Column::Session, &self.meta_key(handle)).await }
    }

    /// Handles from the session index, optionally scoped to one agent.
    fn indexed_handles(
        &self,
        agent: Option<&AgentId>,
    ) -> impl Future<Output = Result<Vec<SessionHandle>>> + Send {
        async move {
            let prefix = match agent {
                Some(a) => self.prefix(&["idx", "sess", &a.to_string()]),
                None => self.prefix(&["idx", "sess"]),
            };
            let rows = self.scan(Column::Session, &prefix).await?;
            Ok(rows
                .into_iter()
                .filter_map(|(_, handle)| String::from_utf8(handle).ok())
                .map(SessionHandle::new)
                .collect())
        }
    }

    /// A session's own words about itself, indexed as two documents so a
    /// summary match can outweigh a title match.
    fn index_meta_text(
        &self,
        handle: &SessionHandle,
        meta: &SessionMeta,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let title_key = self.key(&["session", handle.as_str(), "meta", "title"]);
            match meta.title.is_empty() {
                true => self.drop_text(TextIndex::SessionMeta, &title_key).await?,
                false => {
                    self.index_text(TextIndex::SessionMeta, &title_key, &meta.title, 1.0)
                        .await?
                }
            }
            let summary_key = self.key(&["session", handle.as_str(), "meta", "summary"]);
            match meta.summary.as_deref() {
                None | Some("") => self.drop_text(TextIndex::SessionMeta, &summary_key).await,
                Some(summary) => {
                    self.index_text(TextIndex::SessionMeta, &summary_key, summary, 1.0)
                        .await
                }
            }
        }
    }

    /// Handle → boost, from whichever of a session's title or summary
    /// the query also matched.
    fn meta_boosts(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<BTreeMap<String, f64>>> + Send {
        async move {
            let weights = self.config();
            let mut boosts: BTreeMap<String, f64> = BTreeMap::new();
            for hit in self
                .search_text(TextIndex::SessionMeta, query, limit)
                .await?
            {
                let Ok(key) = std::str::from_utf8(&hit.key) else {
                    continue;
                };
                let mut parts = key.rsplit('/');
                let boost = match parts.next() {
                    Some("title") => weights.title_boost,
                    Some("summary") => weights.summary_boost,
                    _ => continue,
                };
                if parts.next() != Some("meta") {
                    continue;
                }
                let Some(handle) = parts.next() else { continue };
                *boosts.entry(handle.to_owned()).or_insert(0.0) += boost;
            }
            Ok(boosts)
        }
    }

    /// The messages surrounding a hit, bounded by [`MAX_WINDOW_ITEMS`].
    fn window(
        &self,
        handle: &SessionHandle,
        idx: usize,
        opts: &SearchOptions,
    ) -> impl Future<Output = Result<Vec<WindowItem>>> + Send {
        async move {
            let before = opts.context_before.min(MAX_WINDOW_ITEMS);
            let after = opts.context_after.min(MAX_WINDOW_ITEMS);
            let first = idx.saturating_sub(before);
            let last = idx.saturating_add(after);
            let mut out = Vec::new();
            for i in first..=last {
                if out.len() >= MAX_WINDOW_ITEMS {
                    break;
                }
                let key = self.message_key(handle, i);
                let Some(entry) = self.get_json::<HistoryEntry>(Column::Session, &key).await?
                else {
                    continue;
                };
                let (snippet, truncated) = entry.snippet();
                out.push(WindowItem {
                    role: entry.role().clone(),
                    msg_idx: i as u32,
                    snippet,
                    truncated,
                    tool_name: entry.tool_name(),
                });
            }
            Ok(out)
        }
    }
}

impl<T: KVStorage + TextSearch> Sessions for T {}

/// The session and message position a text hit points at. `None` for a
/// key that is not a message.
fn parse_message_key(key: &[u8]) -> Option<(String, usize)> {
    let key = std::str::from_utf8(key).ok()?;
    let mut parts = key.rsplit('/');
    let idx = parts.next()?.parse().ok()?;
    if parts.next()? != "msg" {
        return None;
    }
    Some((parts.next()?.to_owned(), idx))
}

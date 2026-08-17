//! `impl Sessions for Store`, plus the ranking that used to be a query.

use crate::{
    AgentId, HistoryEntry,
    interface::Sessions,
    kv::{Column, KVStorage},
    session::{
        EventLine, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHandle, SessionHit,
        SessionMeta, SessionSnapshot, WindowItem, history::indexable,
    },
    store::Store,
    text::{TextIndex, TextSearch},
};
use anyhow::Result;
use crabllm_core::anthropic::ContentBlock;
use std::collections::BTreeMap;

/// How much a message's own role is worth when ranking it. A person's
/// words say what a session was about; a tool call's name is a weaker
/// signal, and everything else is neutral.
const USER_WEIGHT: f64 = 1.5;
const TOOL_CALL_WEIGHT: f64 = 1.3;

/// Added to a session's best message hit when the query also matches its
/// title or summary. A summary is written about the whole session, so it
/// says more about relevance than a title someone typed once.
const TITLE_BOOST: f64 = 2.0;
const SUMMARY_BOOST: f64 = 3.0;

/// Message matches to pull per requested hit.
///
/// The index ranks messages; a result is a session — its single best
/// message. One session that mentions a term repeatedly would otherwise
/// fill a flat top-N by itself, so over-fetch and group. This is a
/// heuristic bound, not a guarantee: when the candidate set comes back
/// full, the grouping may have had more sessions to choose from, and
/// that is logged rather than passed off as a complete answer.
const CANDIDATES_PER_HIT: usize = 10;

impl<K: KVStorage, T: TextSearch> Sessions for Store<K, T> {
    async fn create_session(&self, agent: &AgentId, created_by: &str) -> Result<SessionHandle> {
        // Opaque identity: the handle encodes nothing, so renaming an
        // agent never orphans its transcripts.
        let handle = SessionHandle::new(ulid::Ulid::new().to_string());
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
        self.put_json(Column::Session, &self.meta_key(&handle), &meta)
            .await?;
        self.kv
            .put(
                Column::Session,
                &self.session_index_key(&meta, &handle),
                handle.as_str().as_bytes(),
            )
            .await?;
        Ok(handle)
    }

    /// The newest session for an identity is the last key under its
    /// prefix — `created_at` is RFC3339 and sorts lexicographically, so
    /// this is a scan and a `last()`, not a query.
    async fn find_latest_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        let prefix = self.session_index_prefix(Some(agent), Some(created_by));
        let rows = self.kv.scan(Column::Session, &prefix).await?;
        Ok(rows
            .last()
            .and_then(|(_, handle)| String::from_utf8(handle.clone()).ok())
            .map(SessionHandle::new))
    }

    async fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let Some(meta) = self.meta(handle).await? else {
            return Ok(None);
        };
        let archive: Option<String> = self
            .get_json(Column::Session, &self.archive_key(handle))
            .await?;

        let keys = self
            .kv
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

    async fn list_sessions(&self) -> Result<Vec<(SessionHandle, SessionMeta)>> {
        let mut out = Vec::new();
        for handle in self.indexed_handles(None).await? {
            if let Some(meta) = self.meta(&handle).await? {
                out.push((handle, meta));
            }
        }
        // Recency ordering is a property of the answer, not of the keys:
        // `updated_at` moves on every append, and an index keyed by it
        // would have to be rewritten each time.
        out.sort_by(|(ah, am), (bh, bm)| {
            bm.updated_at
                .cmp(&am.updated_at)
                .then_with(|| bm.created_at.cmp(&am.created_at))
                .then_with(|| ah.as_str().cmp(bh.as_str()))
        });
        Ok(out)
    }

    async fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let Some(mut meta) = self.meta(handle).await? else {
            anyhow::bail!("session not found: {}", handle.as_str());
        };
        let from = meta.message_count as usize;
        for (offset, entry) in entries.iter().enumerate() {
            let key = self.message_key(handle, from + offset);
            self.put_json(Column::Session, &key, entry).await?;
            // `indexable` decides what may be searched at all — tool
            // results and tool-call arguments are excluded there, so the
            // index never sees a credential that passed through a
            // message.
            if let Some((body, role)) = indexable(entry) {
                let weight = match role {
                    "user" => USER_WEIGHT,
                    "assistant_tool" => TOOL_CALL_WEIGHT,
                    _ => 1.0,
                };
                self.text
                    .index_text(TextIndex::Messages, &key, &body, weight)
                    .await?;
            }
        }
        meta.message_count += entries.len() as u64;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        self.put_json(Column::Session, &self.meta_key(handle), &meta)
            .await
    }

    async fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> Result<()> {
        let prefix = self.tenant.prefix(&["session", handle.as_str(), "evt"]);
        let mut idx = self.kv.scan_keys(Column::Session, &prefix).await?.len();
        for event in events {
            let key = self.event_key(handle, idx);
            self.put_json(Column::Session, &key, event).await?;
            idx += 1;
        }
        Ok(())
    }

    async fn append_session_compact(&self, handle: &SessionHandle, archive: &str) -> Result<()> {
        // The compacted prefix leaves the live history: the marker is a
        // pointer to where the text went, so the messages it covers are
        // dropped rather than kept beside it.
        self.truncate_session_messages(handle, 0).await?;
        self.put_json(Column::Session, &self.archive_key(handle), &archive)
            .await
    }

    async fn truncate_session_messages(&self, handle: &SessionHandle, keep: usize) -> Result<()> {
        let Some(mut meta) = self.meta(handle).await? else {
            return Ok(());
        };
        let keys = self
            .kv
            .scan_keys(Column::Session, &self.message_prefix(handle))
            .await?;
        for key in keys.iter().skip(keep) {
            self.text.drop_text(TextIndex::Messages, key).await?;
            self.kv.delete(Column::Session, key).await?;
        }
        meta.message_count = keep.min(keys.len()) as u64;
        self.put_json(Column::Session, &self.meta_key(handle), &meta)
            .await
    }

    async fn update_session_meta(&self, handle: &SessionHandle, meta: &SessionMeta) -> Result<()> {
        self.put_json(Column::Session, &self.meta_key(handle), meta)
            .await?;
        self.index_meta_text(handle, meta).await
    }

    async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        // Index first. A crash here leaves content nothing can reach,
        // which a sweep collects; the reverse leaves an entry pointing
        // at a transcript that is not there.
        let prefix = self.session_prefix(handle);
        self.text
            .drop_text_prefix(TextIndex::Messages, &prefix)
            .await?;
        self.text
            .drop_text_prefix(TextIndex::SessionMeta, &prefix)
            .await?;
        if let Some(meta) = self.meta(handle).await? {
            self.kv
                .delete(Column::Session, &self.session_index_key(&meta, handle))
                .await?;
        }
        let mut existed = false;
        for key in self.kv.scan_keys(Column::Session, &prefix).await? {
            existed |= self.kv.delete(Column::Session, &key).await?;
        }
        Ok(existed)
    }

    async fn delete_sessions_of(&self, agent: &AgentId) -> Result<usize> {
        let mut purged = 0;
        for handle in self.indexed_handles(Some(agent)).await? {
            if self.delete_session(&handle).await? {
                purged += 1;
            }
        }
        Ok(purged)
    }

    /// Rank messages, keep each session's best, then boost by what the
    /// session as a whole says about itself.
    ///
    /// Three steps because they read different stores: the text index
    /// knows which message matched, KV holds the metadata a hit is
    /// filtered and rendered by, and the window is more KV.
    async fn search_sessions(&self, query: &str, opts: &SearchOptions) -> Result<Vec<SessionHit>> {
        let limit = opts.limit.clamp(1, crate::session::MAX_HITS_PER_QUERY);
        let candidates = limit.saturating_mul(CANDIDATES_PER_HIT);
        let matches = self
            .text
            .search_text(TextIndex::Messages, query, candidates)
            .await?;
        if matches.len() == candidates {
            tracing::debug!(
                "session search hit the candidate ceiling ({candidates}); \
                 some sessions may be missing from the results"
            );
        }

        // Best message per session, in rank order.
        let mut best: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        for hit in &matches {
            let Some((handle, idx)) = self.parse_message_key(&hit.key) else {
                continue;
            };
            best.entry(handle.as_str().to_owned())
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
            let Some(meta) = self.meta(&handle).await? else {
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
            let agent_name = self.load_agent_name(&meta.agent).await.unwrap_or_default();
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

impl<K: KVStorage, T: TextSearch> Store<K, T> {
    /// Handles from the session index, optionally scoped to one agent.
    async fn indexed_handles(&self, agent: Option<&AgentId>) -> Result<Vec<SessionHandle>> {
        let prefix = self.session_index_prefix(agent, None);
        let rows = self.kv.scan(Column::Session, &prefix).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(_, handle)| String::from_utf8(handle).ok())
            .map(SessionHandle::new)
            .collect())
    }

    /// A session's own words about itself, indexed as two documents so a
    /// summary match can outweigh a title match.
    async fn index_meta_text(&self, handle: &SessionHandle, meta: &SessionMeta) -> Result<()> {
        let title_key = self.meta_text_key(handle, "title");
        match meta.title.is_empty() {
            true => {
                self.text
                    .drop_text(TextIndex::SessionMeta, &title_key)
                    .await?
            }
            false => {
                self.text
                    .index_text(TextIndex::SessionMeta, &title_key, &meta.title, 1.0)
                    .await?
            }
        }
        let summary_key = self.meta_text_key(handle, "summary");
        match meta.summary.as_deref() {
            None | Some("") => {
                self.text
                    .drop_text(TextIndex::SessionMeta, &summary_key)
                    .await
            }
            Some(summary) => {
                self.text
                    .index_text(TextIndex::SessionMeta, &summary_key, summary, 1.0)
                    .await
            }
        }
    }

    fn meta_text_key(&self, handle: &SessionHandle, field: &str) -> Vec<u8> {
        self.tenant
            .key(&["session", handle.as_str(), "meta", field])
    }

    /// Handle → boost, from whichever of a session's title or summary
    /// the query also matched.
    async fn meta_boosts(&self, query: &str, limit: usize) -> Result<BTreeMap<String, f64>> {
        let mut boosts: BTreeMap<String, f64> = BTreeMap::new();
        for hit in self
            .text
            .search_text(TextIndex::SessionMeta, query, limit)
            .await?
        {
            let Ok(key) = std::str::from_utf8(&hit.key) else {
                continue;
            };
            let mut parts = key.rsplit('/');
            let boost = match parts.next() {
                Some("title") => TITLE_BOOST,
                Some("summary") => SUMMARY_BOOST,
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

    /// A hit is read by a person or a model, and a bare ULID names
    /// nothing.
    async fn load_agent_name(&self, agent: &AgentId) -> Option<String> {
        let config: Option<crate::AgentConfig> = self
            .get_json(Column::Agent, &self.agent_key(agent))
            .await
            .ok()?;
        config.map(|c| c.name)
    }

    /// The messages surrounding a hit, bounded by [`MAX_WINDOW_ITEMS`].
    async fn window(
        &self,
        handle: &SessionHandle,
        idx: usize,
        opts: &SearchOptions,
    ) -> Result<Vec<WindowItem>> {
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
            let Some(entry) = self.get_json::<HistoryEntry>(Column::Session, &key).await? else {
                continue;
            };
            let (snippet, truncated) = snippet(&entry);
            out.push(WindowItem {
                role: entry.role().clone(),
                msg_idx: i as u32,
                snippet,
                truncated,
                tool_name: tool_name(&entry),
            });
        }
        Ok(out)
    }
}

fn snippet(entry: &HistoryEntry) -> (String, bool) {
    let raw = entry.text().to_owned();
    if raw.len() <= MAX_SNIPPET_BYTES {
        return (raw, false);
    }
    let mut end = MAX_SNIPPET_BYTES;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    (raw[..end].to_owned(), true)
}

fn tool_name(entry: &HistoryEntry) -> Option<String> {
    for block in entry.message.blocks() {
        match block {
            ContentBlock::ToolResult { name: Some(n), .. } if !n.is_empty() => {
                return Some(n.clone());
            }
            ContentBlock::ToolUse { name, .. } => return Some(name.clone()),
            _ => {}
        }
    }
    None
}

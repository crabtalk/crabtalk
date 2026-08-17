//! `impl Sessions for Store`, and the window it reads back from KV.

use crate::{
    AgentId, HistoryEntry,
    interface::Sessions,
    kv::{Column, KVStorage},
    session::{
        EventLine, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHandle, SessionHit,
        SessionMeta, SessionSnapshot, WindowItem,
    },
    sql::{MessageDoc, SqlIndex},
    store::Store,
};
use anyhow::Result;
use crabllm_core::anthropic::ContentBlock;

impl<K: KVStorage, Q: SqlIndex> Store<K, Q> {
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

impl<K: KVStorage, Q: SqlIndex> Sessions for Store<K, Q> {
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
        self.index.index_session(&handle, &meta).await?;
        Ok(handle)
    }

    async fn find_latest_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        self.index.latest_session(agent, created_by).await
    }

    async fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let Some(meta) = self.meta(handle).await? else {
            return Ok(None);
        };
        let archive: Option<String> = self
            .get_json(Column::Session, &self.archive_key(handle))
            .await?;

        let prefix = self.tenant.prefix(&["session", handle.as_str(), "msg"]);
        let keys = self.kv.scan_keys(Column::Session, &prefix).await?;
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
        self.index.session_rows().await
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
        }
        meta.message_count += entries.len() as u64;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        self.put_json(Column::Session, &self.meta_key(handle), &meta)
            .await?;

        // `indexable` decides what may be searched at all — tool results
        // and tool-call arguments are excluded there, so the index never
        // sees a credential that passed through a message.
        let docs: Vec<MessageDoc> = entries
            .iter()
            .enumerate()
            .filter_map(|(offset, entry)| {
                let (body, role) = crate::session::history::indexable(entry)?;
                Some(MessageDoc {
                    idx: from + offset,
                    role: role.to_owned(),
                    body,
                })
            })
            .collect();
        self.index.index_messages(handle, &docs).await?;
        self.index.index_session(handle, &meta).await
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
        let prefix = self.tenant.prefix(&["session", handle.as_str(), "msg"]);
        let keys = self.kv.scan_keys(Column::Session, &prefix).await?;
        for key in keys.iter().skip(keep) {
            self.kv.delete(Column::Session, key).await?;
        }
        self.index.drop_messages_from(handle, keep).await?;
        meta.message_count = keep.min(keys.len()) as u64;
        self.put_json(Column::Session, &self.meta_key(handle), &meta)
            .await?;
        self.index.index_session(handle, &meta).await
    }

    async fn update_session_meta(&self, handle: &SessionHandle, meta: &SessionMeta) -> Result<()> {
        self.put_json(Column::Session, &self.meta_key(handle), meta)
            .await?;
        self.index.index_session(handle, meta).await
    }

    async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        // Index first. A crash here leaves content nothing can reach,
        // which a sweep collects; the reverse leaves a row pointing at a
        // transcript that is not there.
        let indexed = self.index.unindex_session(handle).await?;
        let prefix = self.tenant.prefix(&["session", handle.as_str()]);
        for key in self.kv.scan_keys(Column::Session, &prefix).await? {
            self.kv.delete(Column::Session, &key).await?;
        }
        let stored = self
            .kv
            .delete(Column::Session, &self.meta_key(handle))
            .await?;
        Ok(indexed || stored)
    }

    async fn delete_sessions_of(&self, agent: &AgentId) -> Result<usize> {
        let handles = self.index.session_handles_of(agent).await?;
        let mut purged = 0;
        for handle in &handles {
            if self.delete_session(handle).await? {
                purged += 1;
            }
        }
        Ok(purged)
    }

    /// Search the index, then fill each hit's window from KV.
    ///
    /// Two steps because they read different stores: the index knows
    /// which message matched, and only KV has the messages around it.
    async fn search_sessions(&self, query: &str, opts: &SearchOptions) -> Result<Vec<SessionHit>> {
        let mut hits = self.index.search_messages(query, opts).await?;
        for hit in &mut hits {
            hit.window = self
                .window(&hit.session_handle, hit.msg_idx as usize, opts)
                .await?;
        }
        Ok(hits)
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

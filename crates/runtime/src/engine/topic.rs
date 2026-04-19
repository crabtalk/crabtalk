//! Topics — per-(agent, sender) conversation partitioning. See
//! [RFC 0171](https://github.com/crabtalk/crabtalk/issues/171). One
//! `(agent, sender)` pair maps to N conversations keyed by topic
//! title, plus an active-topic pointer. Untopicked chats are tmp and
//! live only in [`TopicRouter::tmp`]; they never reach storage.

use super::Runtime;
use crate::{Config, ConversationHandle};
use anyhow::{Result, bail};
use memory::{EntryKind, Op};
use std::{collections::HashMap, sync::atomic::Ordering};
use wcore::storage::Storage;

/// Per-(agent, sender) topic routing. `active = None` means the caller
/// is on a tmp chat (no topic). Tmp chats have their own `ConvSlot` but
/// their id is tracked here so `get_or_create_conversation` can find
/// them without scanning.
#[derive(Default)]
pub struct TopicRouter {
    pub(super) by_title: HashMap<String, u64>,
    pub(super) active: Option<String>,
    pub(super) tmp: Option<u64>,
}

impl TopicRouter {
    /// Resolve the conversation this router currently routes to:
    /// the active topic's conversation if one is set, otherwise the
    /// tmp conversation if one exists.
    pub(super) fn active_conversation(&self) -> Option<u64> {
        self.active
            .as_ref()
            .and_then(|t| self.by_title.get(t).copied())
            .or(self.tmp)
    }
}

/// Outcome of `switch_topic`. `resumed = true` means the topic
/// already existed (in the router or on disk); `false` means it was
/// freshly created.
#[derive(Debug, Clone, Copy)]
pub struct SwitchOutcome {
    pub conversation_id: u64,
    pub resumed: bool,
}

impl<C: Config> Runtime<C> {
    /// Switch the active topic for `(agent, sender)`. Creates a new
    /// topic conversation if the title doesn't exist yet; resumes the
    /// existing one otherwise. When creating, writes an
    /// `EntryKind::Topic` memory entry (unless one already exists for
    /// the title) so the topic is searchable via `search_topics`.
    ///
    /// Returns the target `conversation_id` in the outcome. The caller
    /// is responsible for telling the user which conversation to route
    /// the next message to — this call only updates runtime state.
    pub async fn switch_topic(
        &self,
        agent: &str,
        sender: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<SwitchOutcome> {
        if !self.has_agent(agent).await {
            bail!("agent '{agent}' not registered");
        }
        if title.is_empty() {
            bail!("topic title cannot be empty");
        }

        let key = (agent.to_owned(), sender.to_owned());

        // Reserve the slot under the router lock — any concurrent
        // caller that races us observes the reservation on the next
        // lookup and resumes to our conversation instead of creating a
        // duplicate session.
        let id = {
            let mut topics = self.topics.write().await;
            let router = topics.entry(key.clone()).or_default();
            if let Some(id) = router.by_title.get(title).copied() {
                router.active = Some(title.to_owned());
                return Ok(SwitchOutcome {
                    conversation_id: id,
                    resumed: true,
                });
            }
            let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
            router.by_title.insert(title.to_owned(), id);
            router.active = Some(title.to_owned());
            id
        };

        match self
            .finalize_switch(agent, sender, title, description, id)
            .await
        {
            Ok(outcome) => Ok(outcome),
            Err(e) => {
                self.rollback_reservation(&key, title).await;
                Err(e)
            }
        }
    }

    /// Cold-path body of `switch_topic`. Called after the slot
    /// has been reserved; any error here triggers a router rollback.
    async fn finalize_switch(
        &self,
        agent: &str,
        sender: &str,
        title: &str,
        description: Option<&str>,
        id: u64,
    ) -> Result<SwitchOutcome> {
        let existing = self.find_session(agent, sender, title);
        let resumed = existing.is_some();

        if !resumed {
            let desc = description.ok_or_else(|| {
                anyhow::anyhow!("description required when creating a new topic '{title}'")
            })?;
            self.ensure_entry(title, desc);
        }

        let slot = Self::new_slot(id, agent, sender);
        {
            let mut conversation = slot.inner.lock().await;
            conversation.topic = Some(title.to_owned());
            match existing {
                Some((handle, snapshot)) => {
                    conversation.history =
                        self.resumed_history(snapshot.archive.as_deref(), snapshot.history);
                    conversation.title = snapshot.meta.title;
                    conversation.uptime_secs = snapshot.meta.uptime_secs;
                    conversation.handle = Some(handle);
                }
                None => {
                    let storage = self.storage();
                    let handle = storage.create_session(agent, sender)?;
                    // Stamp meta so the new session carries its topic
                    // from the first write — a missing `topic` here
                    // would make the session invisible to future
                    // `find_session` scans. `conversation.meta`
                    // already reflects the topic we set above.
                    storage.update_session_meta(&handle, &conversation.meta(agent, sender))?;
                    conversation.handle = Some(handle);
                }
            }
        }

        self.conversations.write().await.insert(id, slot);
        Ok(SwitchOutcome {
            conversation_id: id,
            resumed,
        })
    }

    async fn rollback_reservation(&self, key: &(String, String), title: &str) {
        let mut topics = self.topics.write().await;
        let Some(router) = topics.get_mut(key) else {
            return;
        };
        router.by_title.remove(title);
        if router.active.as_deref() == Some(title) {
            router.active = None;
        }
        if router.by_title.is_empty() && router.active.is_none() && router.tmp.is_none() {
            topics.remove(key);
        }
    }

    /// Scan storage for a session matching `(agent, sender, topic)`.
    /// Blocking I/O; call from outside the runtime locks.
    fn find_session(
        &self,
        agent: &str,
        sender: &str,
        title: &str,
    ) -> Option<(ConversationHandle, wcore::storage::SessionSnapshot)> {
        let storage = self.storage();
        let summaries = storage.list_sessions().ok()?;
        let mut best: Option<(ConversationHandle, wcore::storage::ConversationMeta)> = None;
        for summary in summaries {
            if summary.meta.agent != agent || summary.meta.created_by != sender {
                continue;
            }
            if summary.meta.topic.as_deref() != Some(title) {
                continue;
            }
            // Later sessions win — `created_at` is an RFC3339 string so
            // lexicographic comparison is chronological.
            if best
                .as_ref()
                .is_none_or(|(_, meta)| summary.meta.created_at > meta.created_at)
            {
                best = Some((summary.handle, summary.meta));
            }
        }
        let (handle, _) = best?;
        let snapshot = storage.load_session(&handle).ok().flatten()?;
        Some((handle, snapshot))
    }

    /// Write the Topic memory entry if it doesn't already exist.
    /// Duplicate-name errors are ignored — a prior process may have
    /// created the same topic.
    fn ensure_entry(&self, title: &str, description: &str) {
        let mut mem = self.memory.write();
        if mem.get(title).is_some() {
            return;
        }
        if let Err(e) = mem.apply(Op::Add {
            name: title.to_owned(),
            content: description.to_owned(),
            aliases: vec![],
            kind: EntryKind::Topic,
        }) {
            tracing::warn!("topic entry write failed: {e}");
        }
    }
}

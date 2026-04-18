//! Conversation management — lifecycle, persistence, and title generation.

use crate::{Config, Conversation};
use anyhow::{Result, bail};
use crabllm_core::{ChatCompletionRequest, Message, Role};
use memory::{EntryKind, Op};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::Mutex;
use wcore::{
    model::HistoryEntry,
    storage::{SessionHandle, Storage},
};

use super::{ConvSlot, Runtime, TopicRouter};

/// Outcome of `switch_active_topic`. `resumed = true` means the topic
/// already existed (in the router or on disk); `false` means it was
/// freshly created.
#[derive(Debug, Clone, Copy)]
pub struct SwitchOutcome {
    pub conversation_id: u64,
    pub resumed: bool,
}

impl<C: Config> Runtime<C> {
    fn new_slot(id: u64, agent: &str, created_by: &str) -> ConvSlot {
        ConvSlot {
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            inner: Arc::new(Mutex::new(Conversation::new(id))),
        }
    }

    /// Get or create a conversation for the given (agent, created_by)
    /// identity. Routing order:
    ///
    /// 1. If `(agent, sender)` has an active topic, return that topic's
    ///    conversation.
    /// 2. Otherwise return/create the tmp conversation for this pair —
    ///    in-memory only, no storage I/O, no resume. Topic-bound chats
    ///    reach storage via `switch_active_topic`.
    pub async fn get_or_create_conversation(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.has_agent(agent).await {
            bail!("agent '{agent}' not registered");
        }

        let key = (agent.to_owned(), created_by.to_owned());

        // Read-first: the common case is a hit on an existing tmp or
        // active-topic conversation. Avoid allocating a `TopicRouter`
        // until we actually need to insert one.
        if let Some(id) = self
            .topics
            .read()
            .await
            .get(&key)
            .and_then(TopicRouter::active_conversation)
        {
            return Ok(id);
        }

        let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
        let slot = Self::new_slot(id, agent, created_by);
        let mut topics = self.topics.write().await;
        let router = topics.entry(key).or_default();
        // Re-check under write lock in case another task raced us.
        if let Some(existing) = router.active_conversation() {
            return Ok(existing);
        }
        router.tmp = Some(id);
        drop(topics);
        self.conversations.write().await.insert(id, slot);
        Ok(id)
    }

    /// Switch the active topic for `(agent, sender)`. Creates a new
    /// topic conversation if the title doesn't exist yet; resumes the
    /// existing one otherwise. When creating, writes an
    /// `EntryKind::Topic` memory entry (unless one already exists for
    /// the title) so the topic is searchable via `search_topics`.
    ///
    /// Returns the target `conversation_id` in the outcome. The caller
    /// is responsible for telling the user which conversation to route
    /// the next message to — this call only updates runtime state.
    pub async fn switch_active_topic(
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
            .finalize_topic_switch(agent, sender, title, description, id)
            .await
        {
            Ok(outcome) => Ok(outcome),
            Err(e) => {
                self.rollback_topic_reservation(&key, title).await;
                Err(e)
            }
        }
    }

    /// Cold-path body of `switch_active_topic`. Called after the slot
    /// has been reserved; any error here triggers a router rollback.
    async fn finalize_topic_switch(
        &self,
        agent: &str,
        sender: &str,
        title: &str,
        description: Option<&str>,
        id: u64,
    ) -> Result<SwitchOutcome> {
        let existing = self.find_topic_session(agent, sender, title);
        let resumed = existing.is_some();

        if !resumed {
            let desc = description.ok_or_else(|| {
                anyhow::anyhow!("description required when creating a new topic '{title}'")
            })?;
            self.ensure_topic_entry(title, desc);
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
                    // `find_topic_session` scans. `conversation.meta`
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

    async fn rollback_topic_reservation(&self, key: &(String, String), title: &str) {
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
    fn find_topic_session(
        &self,
        agent: &str,
        sender: &str,
        title: &str,
    ) -> Option<(SessionHandle, wcore::storage::SessionSnapshot)> {
        let storage = self.storage();
        let summaries = storage.list_sessions().ok()?;
        let mut match_: Option<(SessionHandle, wcore::storage::ConversationMeta)> = None;
        for summary in summaries {
            if summary.meta.agent != agent || summary.meta.created_by != sender {
                continue;
            }
            if summary.meta.topic.as_deref() != Some(title) {
                continue;
            }
            // Later sessions win — `created_at` is an RFC3339 string so
            // lexicographic comparison is chronological.
            if match_
                .as_ref()
                .is_none_or(|(_, meta)| summary.meta.created_at > meta.created_at)
            {
                match_ = Some((summary.handle, summary.meta));
            }
        }
        let (handle, _) = match_?;
        let snapshot = storage.load_session(&handle).ok().flatten()?;
        Some((handle, snapshot))
    }

    /// Write the Topic memory entry if it doesn't already exist.
    /// Duplicate-name errors are ignored — a prior process may have
    /// created the same topic.
    fn ensure_topic_entry(&self, title: &str, description: &str) {
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

    /// Load a specific conversation by session handle.
    pub async fn load_specific_conversation(&self, handle: SessionHandle) -> Result<u64> {
        let storage = self.storage();
        let snapshot = storage
            .load_session(&handle)?
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", handle.as_str()))?;
        if !self.has_agent(&snapshot.meta.agent).await {
            bail!("agent '{}' not registered", snapshot.meta.agent);
        }
        let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
        let slot = Self::new_slot(id, &snapshot.meta.agent, &snapshot.meta.created_by);
        let mut conversation = slot.inner.lock().await;
        conversation.history = self.resumed_history(snapshot.archive.as_deref(), snapshot.history);
        conversation.title = snapshot.meta.title;
        conversation.uptime_secs = snapshot.meta.uptime_secs;
        conversation.handle = Some(handle);
        drop(conversation);
        self.conversations.write().await.insert(id, slot);
        Ok(id)
    }

    pub async fn active_conversation_infos(
        &self,
    ) -> Vec<wcore::protocol::message::ActiveConversationInfo> {
        let conversations = self.conversations.read().await;
        let mut infos = Vec::with_capacity(conversations.len());
        for (_, conv_slot) in conversations.iter() {
            let c = conv_slot.inner.lock().await;
            infos.push(wcore::protocol::message::ActiveConversationInfo {
                agent: conv_slot.agent.clone(),
                sender: conv_slot.created_by.clone(),
                message_count: c.history.len() as u64,
                alive_secs: c.uptime_secs,
                title: c.title.clone(),
            });
        }
        infos
    }

    pub async fn close_conversation(&self, id: u64) -> bool {
        self.steering.write().await.remove(&id);
        let removed = self.conversations.write().await.remove(&id);
        if let Some(slot) = &removed {
            let key = (slot.agent.clone(), slot.created_by.clone());
            let mut topics = self.topics.write().await;
            if let Some(router) = topics.get_mut(&key) {
                if router.tmp == Some(id) {
                    router.tmp = None;
                }
                router.by_title.retain(|_, cid| *cid != id);
                if router
                    .active
                    .as_ref()
                    .is_some_and(|t| !router.by_title.contains_key(t))
                {
                    router.active = None;
                }
                if router.tmp.is_none() && router.by_title.is_empty() {
                    topics.remove(&key);
                }
            }
        }
        removed.is_some()
    }

    pub async fn steer(&self, conversation_id: u64, content: String) -> Result<()> {
        let senders = self.steering.read().await;
        let tx = senders.get(&conversation_id).ok_or_else(|| {
            anyhow::anyhow!("no active stream for conversation {conversation_id}")
        })?;
        tx.send(Some(content))
            .map_err(|_| anyhow::anyhow!("steering channel closed"))?;
        Ok(())
    }

    pub async fn conversation(&self, id: u64) -> Option<Arc<Mutex<Conversation>>> {
        self.conversations
            .read()
            .await
            .get(&id)
            .map(|slot| slot.inner.clone())
    }

    /// Look up a conversation slot's `(agent, sender, mutex)` triple.
    /// Returns `None` when the conversation id is not registered — the
    /// execution paths all need this handshake before locking the
    /// conversation mutex.
    pub(crate) async fn acquire_slot(
        &self,
        id: u64,
    ) -> Option<(String, String, Arc<Mutex<Conversation>>)> {
        self.conversations
            .read()
            .await
            .get(&id)
            .map(ConvSlot::parts)
    }

    pub async fn conversations(&self) -> Vec<Arc<Mutex<Conversation>>> {
        self.conversations
            .read()
            .await
            .values()
            .map(|slot| slot.inner.clone())
            .collect()
    }

    pub async fn conversation_count(&self) -> usize {
        self.conversations.read().await.len()
    }

    pub async fn find_conversation_id(&self, agent: &str, sender: &str) -> Option<u64> {
        let conversations = self.conversations.read().await;
        for (id, slot) in conversations.iter() {
            if slot.agent == agent && slot.created_by == sender {
                return Some(*id);
            }
        }
        None
    }

    pub async fn compact_conversation(&self, conversation_id: u64) -> Option<String> {
        let (agent_name, history) = {
            let conversations = self.conversations.read().await;
            let slot = conversations.get(&conversation_id)?;
            let agent_name = slot.agent.clone();
            let conversation_mutex = slot.inner.clone();
            drop(conversations);
            let conversation = conversation_mutex.lock().await;
            if conversation.history.is_empty() {
                return None;
            }
            (agent_name, conversation.history.clone())
        };
        self.resolve_agent(&agent_name)
            .await?
            .compact(&history)
            .await
    }

    pub async fn transfer_conversations<C2: Config>(&self, dest: &mut Runtime<C2>) {
        let conversations = self.conversations.read().await;
        let dest_conversations = dest.conversations.get_mut();
        for (id, slot) in conversations.iter() {
            dest_conversations.insert(*id, slot.clone());
        }
        let next = self.next_conversation_id.load(Ordering::Relaxed);
        dest.next_conversation_id.store(next, Ordering::Relaxed);
    }

    /// Build the conversation's replay history from storage's post-compact
    /// messages plus the Archive entry's content. A missing archive entry
    /// (memory wiped, different machine, etc.) injects a visible placeholder
    /// so the model can acknowledge the gap instead of silently truncating
    /// the user's context.
    fn resumed_history(
        &self,
        archive: Option<&str>,
        mut history: Vec<HistoryEntry>,
    ) -> Vec<HistoryEntry> {
        let Some(name) = archive else { return history };
        let content = {
            let mem = self.memory.read();
            mem.get(name).map(|e| e.content.clone())
        };
        let prefix = content.unwrap_or_else(|| {
            tracing::warn!("resume: archive '{name}' missing from memory");
            format!("[archived context unavailable: {name}]")
        });
        let mut out = Vec::with_capacity(history.len() + 1);
        out.push(HistoryEntry::user(prefix));
        out.append(&mut history);
        out
    }

    /// Write a compaction summary to memory as an `Archive` entry,
    /// named `{topic-slug}-{n}` where `n` is the next free sequence
    /// number for this topic. Older archives stay searchable via
    /// `recall`, so a long-running topic's phases don't get
    /// overwritten. Returns the generated name, or `None` on failure
    /// — the caller must skip the compact marker so a resume can't
    /// dangle.
    fn write_archive(&self, topic: &str, summary: String) -> Option<String> {
        let slug = wcore::sender_slug(topic);
        let prefix = format!("{slug}-");
        let mut mem = self.memory.write();
        // Scan and insert under the same write lock — two concurrent
        // compactions can't both pick `seq` and collide.
        let next_seq = mem
            .list()
            .filter(|e| e.kind == EntryKind::Archive && e.name.starts_with(&prefix))
            .filter_map(|e| {
                let suffix = &e.name[prefix.len()..];
                let n: u32 = suffix.parse().ok()?;
                // Reject non-canonical forms ("02", "+1", etc.) so a
                // future `{slug}-2` can't collide with a historic
                // `{slug}-02`.
                (n.to_string() == suffix).then_some(n)
            })
            .max()
            .unwrap_or(0)
            + 1;
        let name = format!("{slug}-{next_seq}");
        match mem.apply(Op::Add {
            name: name.clone(),
            content: summary,
            aliases: vec![],
            kind: EntryKind::Archive,
        }) {
            Ok(()) => Some(name),
            Err(e) => {
                tracing::error!("archive write failed: {e}");
                None
            }
        }
    }

    /// Ensure a topic-bound conversation has a session handle, creating
    /// one via the repo if needed. Tmp chats (no topic) are in-memory
    /// only — never persisted, never assigned a handle.
    fn ensure_handle(&self, conversation: &mut Conversation, agent: &str, created_by: &str) {
        if conversation.handle.is_some() || conversation.topic.is_none() {
            return;
        }
        let storage = self.storage();
        match storage.create_session(agent, created_by) {
            Ok(handle) => conversation.handle = Some(handle),
            Err(e) => tracing::warn!("failed to create session: {e}"),
        }
    }

    /// Post-run tail shared by `send_to`, `stream_to`, and
    /// `guest_stream_to`: update uptime, persist, and kick off title
    /// generation if the conversation has a titleable exchange and no
    /// title yet.
    pub(crate) fn finalize_run(
        &self,
        conversation_id: u64,
        conversation: &mut Conversation,
        conversation_mutex: Arc<Mutex<Conversation>>,
        agent: &str,
        created_by: &str,
        run_start: std::time::Instant,
        pre_run_len: usize,
        compact_summary: Option<String>,
        event_trace: &[wcore::EventLine],
    ) {
        conversation.uptime_secs += run_start.elapsed().as_secs();
        self.persist_messages(
            conversation,
            agent,
            created_by,
            pre_run_len,
            compact_summary,
            event_trace,
        );
        if conversation.title.is_empty() && conversation.history.len() >= 2 {
            self.spawn_title_generation(conversation_id, agent, created_by, conversation_mutex);
        }
    }

    /// Persist messages to the session repo. Handles ensure_handle,
    /// compact markers, and meta updates.
    pub(crate) fn persist_messages(
        &self,
        conversation: &mut Conversation,
        agent: &str,
        created_by: &str,
        pre_run_len: usize,
        compact_summary: Option<String>,
        event_trace: &[wcore::EventLine],
    ) {
        self.ensure_handle(conversation, agent, created_by);
        let Some(ref handle) = conversation.handle else {
            return;
        };
        let storage = self.storage();

        if let Some(summary) = compact_summary {
            // A persisted conversation is always topic-bound —
            // `ensure_handle` refuses to create a handle for tmp chats.
            let topic = conversation
                .topic
                .clone()
                .expect("persisted conversation without a topic");
            // Archive first — if this fails, don't write a dangling
            // marker that points at nothing.
            if let Some(archive_name) = self.write_archive(&topic, summary) {
                let _ = storage.append_session_compact(handle, &archive_name);
                if conversation.history.len() > 1 {
                    let tail: Vec<_> = conversation.history[1..]
                        .iter()
                        .filter(|e| !e.auto_injected)
                        .cloned()
                        .collect();
                    let _ = storage.append_session_messages(handle, &tail);
                }
            }
        } else {
            let new_entries: Vec<_> = conversation.history[pre_run_len..]
                .iter()
                .filter(|e| !e.auto_injected)
                .cloned()
                .collect();
            let _ = storage.append_session_messages(handle, &new_entries);
        }
        if !event_trace.is_empty() {
            let _ = storage.append_session_events(handle, event_trace);
        }
        let _ = storage.update_session_meta(handle, &conversation.meta(agent, created_by));
    }

    pub(crate) fn spawn_title_generation(
        &self,
        _conversation_id: u64,
        agent_name: &str,
        created_by: &str,
        conversation_mutex: Arc<Mutex<Conversation>>,
    ) {
        let model = self.model.clone();
        let storage = self.storage().clone();
        let agent_name = agent_name.to_owned();
        let created_by = created_by.to_owned();
        let model_name = self
            .agents
            .read()
            .get(agent_name.as_str())
            .and_then(|a| a.config.model.clone())
            .unwrap_or_default();
        if model_name.is_empty() {
            return;
        }
        tokio::spawn(async move {
            let (user_msg, assistant_msg) = {
                let conversation = conversation_mutex.lock().await;
                let user = conversation
                    .history
                    .iter()
                    .find(|e| *e.role() == Role::User && !e.auto_injected)
                    .map(|e| e.text().to_owned());
                let assistant = conversation
                    .history
                    .iter()
                    .find(|e| *e.role() == Role::Assistant)
                    .map(|e| e.text().to_owned());
                (user, assistant)
            };

            let Some(user) = user_msg else { return };
            let Some(assistant) = assistant_msg else {
                return;
            };

            let user_snippet: String = user.chars().take(200).collect();
            let assistant_snippet: String = assistant.chars().take(200).collect();

            let prompt = format!(
                "Summarize this conversation in 3-6 words as a short title. \
                 Return ONLY the title, nothing else.\n\n\
                 User: {user_snippet}\nAssistant: {assistant_snippet}"
            );

            let request = ChatCompletionRequest {
                model: model_name,
                messages: vec![Message::user(&prompt)],
                temperature: None,
                top_p: None,
                max_tokens: None,
                stream: None,
                stop: None,
                tools: None,
                tool_choice: None,
                frequency_penalty: None,
                presence_penalty: None,
                seed: None,
                user: None,
                reasoning_effort: None,
                extra: Default::default(),
            };

            match model.send_ct(request).await {
                Ok(response) => {
                    if let Some(title) = response.content() {
                        let title = title.trim().trim_matches('"').to_string();
                        if !title.is_empty() {
                            let mut conversation = conversation_mutex.lock().await;
                            if conversation.title.is_empty() {
                                conversation.title = title;
                                if let Some(ref handle) = conversation.handle {
                                    let _ = storage.update_session_meta(
                                        handle,
                                        &conversation.meta(&agent_name, &created_by),
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("title generation failed: {e}");
                }
            }
        });
    }
}

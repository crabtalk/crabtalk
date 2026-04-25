//! Conversation management — lifecycle, persistence, and title generation.

use super::{ConvSlot, Runtime};
use crate::{Config, Conversation, ConversationHandle};
use anyhow::{Result, bail};
use crabllm_core::{ChatCompletionRequest, Message, Role};
use memory::{EntryKind, Op};
use std::sync::{Arc, atomic::Ordering};
use tokio::sync::Mutex;
use wcore::{model::HistoryEntry, storage::Storage};

impl<C: Config> Runtime<C> {
    pub(super) fn new_slot(id: u64, agent: &str, created_by: &str) -> ConvSlot {
        ConvSlot {
            agent: agent.to_owned(),
            created_by: created_by.to_owned(),
            inner: Arc::new(Mutex::new(Conversation::new(id))),
        }
    }

    /// Get or create a conversation for the given (agent, created_by)
    /// identity. Returns the existing conversation for this pair if one
    /// is active, otherwise allocates a fresh id and persists.
    pub async fn get_or_create_conversation(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.has_agent(agent).await {
            bail!("agent '{agent}' not registered");
        }

        // Read-first: a single active conversation per (agent, sender)
        // is the common case; only fall through to allocation if none
        // is registered.
        if let Some(id) = self.conversation_id(agent, created_by).await {
            return Ok(id);
        }

        let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
        let slot = Self::new_slot(id, agent, created_by);
        self.conversations.write().await.insert(id, slot);
        Ok(id)
    }

    /// Load a specific conversation by persistent handle.
    pub async fn load(&self, handle: ConversationHandle) -> Result<u64> {
        let storage = self.storage();
        let snapshot = storage
            .load_session(&handle)?
            .ok_or_else(|| anyhow::anyhow!("conversation '{}' not found", handle.as_str()))?;
        if !self.has_agent(&snapshot.meta.agent).await {
            bail!("agent '{}' not registered", snapshot.meta.agent);
        }
        let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
        let slot = Self::new_slot(id, &snapshot.meta.agent, &snapshot.meta.created_by);
        {
            let mut conversation = slot.inner.lock().await;
            conversation.history =
                self.resumed_history(snapshot.archive.as_deref(), snapshot.history);
            conversation.title = snapshot.meta.title;
            conversation.uptime_secs = snapshot.meta.uptime_secs;
            conversation.handle = Some(handle);
        }
        self.conversations.write().await.insert(id, slot);
        Ok(id)
    }

    pub async fn list_active(&self) -> Vec<wcore::protocol::message::ActiveConversationInfo> {
        // Snapshot the slot metadata and mutex handles first so the
        // outer read guard isn't held across per-conversation locks —
        // otherwise a slow conversation would block readers of the
        // whole map.
        let slots: Vec<_> = {
            let conversations = self.conversations.read().await;
            conversations
                .values()
                .map(|s| (s.agent.clone(), s.created_by.clone(), s.inner.clone()))
                .collect()
        };
        let mut infos = Vec::with_capacity(slots.len());
        for (agent, sender, mutex) in slots {
            let c = mutex.lock().await;
            infos.push(wcore::protocol::message::ActiveConversationInfo {
                agent,
                sender,
                message_count: c.history.len() as u64,
                alive_secs: c.uptime_secs,
                title: c.title.clone(),
            });
        }
        infos
    }

    pub async fn close(&self, id: u64) -> bool {
        self.steering.write().await.remove(&id);
        self.conversations.write().await.remove(&id).is_some()
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

    pub async fn conversation_id(&self, agent: &str, sender: &str) -> Option<u64> {
        let conversations = self.conversations.read().await;
        for (id, slot) in conversations.iter() {
            if slot.agent == agent && slot.created_by == sender {
                return Some(*id);
            }
        }
        None
    }

    /// Look up a conversation id by (agent, sender) or return a
    /// user-visible error. Shared error-text for endpoints that dispatch
    /// by conversation identity.
    pub async fn require_conversation_id(&self, agent: &str, sender: &str) -> Result<u64> {
        self.conversation_id(agent, sender).await.ok_or_else(|| {
            anyhow::anyhow!("conversation not found for agent='{agent}' sender='{sender}'")
        })
    }

    /// Compact the conversation identified by (agent, sender). Errors if
    /// no conversation exists for that pair or if compaction fails (empty
    /// history, agent gone, etc.).
    pub async fn compact_conversation(&self, agent: &str, sender: &str) -> Result<String> {
        let id = self.require_conversation_id(agent, sender).await?;
        self.compact(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("compact failed for agent='{agent}' sender='{sender}'"))
    }

    /// Steer the conversation identified by (agent, sender) with the given
    /// content. Errors if no conversation exists for that pair.
    pub async fn steer_conversation(
        &self,
        agent: &str,
        sender: &str,
        content: String,
    ) -> Result<()> {
        let id = self.require_conversation_id(agent, sender).await?;
        self.steer(id, content).await
    }

    pub async fn compact(&self, conversation_id: u64) -> Option<String> {
        // Release the conversations read lock before the per-conversation
        // mutex await — otherwise readers queue behind a potentially
        // contended inner lock.
        let (agent_name, conversation_mutex) = {
            let conversations = self.conversations.read().await;
            let slot = conversations.get(&conversation_id)?;
            (slot.agent.clone(), slot.inner.clone())
        };
        let history = {
            let conversation = conversation_mutex.lock().await;
            if conversation.history.is_empty() {
                return None;
            }
            conversation.history.clone()
        };
        self.resolve_agent(&agent_name)
            .await?
            .compact(&history)
            .await
    }

    pub async fn transfer_to<C2: Config>(&self, dest: &mut Runtime<C2>) {
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
    pub(super) fn resumed_history(
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
    /// named `{session-slug}-{n}` where `n` is the next free sequence
    /// number for this session. Older archives stay searchable via
    /// `recall`, so a long-running session's phases don't get
    /// overwritten. Returns the generated name, or `None` on failure
    /// — the caller must skip the compact marker so a resume can't
    /// dangle.
    fn write_archive(&self, session_slug: &str, summary: String) -> Option<String> {
        let slug = wcore::sender_slug(session_slug);
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

    /// Ensure the conversation has a session handle, creating one via
    /// the repo if needed. Sessions persist unconditionally — every
    /// conversation reaches storage on first persist call.
    fn ensure_handle(&self, conversation: &mut Conversation, agent: &str, created_by: &str) {
        if conversation.handle.is_some() {
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
    #[allow(clippy::too_many_arguments)]
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
            // Archive first — if this fails, don't write a dangling
            // marker that points at nothing. Archives are keyed by the
            // session's storage slug so each session's phases stay
            // grouped under one prefix in memory.
            if let Some(archive_name) = self.write_archive(handle.as_str(), summary) {
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
            .map(|a| a.config.model.clone())
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
                thinking: None,
                anthropic_max_tokens: None,
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

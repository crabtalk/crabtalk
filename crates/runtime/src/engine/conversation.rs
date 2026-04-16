//! Conversation management — lifecycle, persistence, and title generation.

use crate::{Config, Conversation};
use anyhow::{Result, bail};
use crabllm_core::{ChatCompletionRequest, Message, Role};
use memory::{EntryKind, Op};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use wcore::{
    model::HistoryEntry,
    storage::{SessionHandle, Storage},
};

use super::Runtime;

fn archive_base_name(session_slug: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("archive-{session_slug}-{nanos}")
}

impl<C: Config> Runtime<C> {
    fn conversation_key(agent: &str, created_by: &str) -> (String, String) {
        (agent.to_owned(), created_by.to_owned())
    }

    fn index_conversation(
        index: &mut std::collections::BTreeMap<(String, String), u64>,
        id: u64,
        agent: &str,
        created_by: &str,
    ) {
        index
            .entry(Self::conversation_key(agent, created_by))
            .or_insert(id);
    }

    async fn find_conversation_id_slow(&self, agent: &str, sender: &str) -> Option<u64> {
        let conversations = self.conversations.read().await;
        for (id, conv_mutex) in conversations.iter() {
            let conv = conv_mutex.lock().await;
            if conv.agent == agent && conv.created_by == sender {
                return Some(*id);
            }
        }
        None
    }

    /// Get or create a conversation for the given (agent, created_by) identity.
    pub async fn get_or_create_conversation(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.has_agent(agent).await {
            bail!("agent '{agent}' not registered");
        }

        // 1. In-memory lookup.
        if let Some(id) = self.find_conversation_id(agent, created_by).await {
            return Ok(id);
        }

        // 2. Storage lookup — find latest persisted session for this identity.
        let storage = self.storage();
        if let Ok(Some(handle)) = storage.find_latest_session(agent, created_by)
            && let Ok(Some(snapshot)) = storage.load_session(&handle)
        {
            let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
            let mut conversation = Conversation::new(id, agent, created_by);
            conversation.history =
                self.resumed_history(snapshot.archive.as_deref(), snapshot.history);
            conversation.title = snapshot.meta.title;
            conversation.uptime_secs = snapshot.meta.uptime_secs;
            conversation.handle = Some(handle);
            self.conversations
                .write()
                .await
                .insert(id, Arc::new(Mutex::new(conversation)));
            let mut index = self.conversation_index.write().await;
            Self::index_conversation(&mut index, id, agent, created_by);
            return Ok(id);
        }

        // 3. Create new.
        self.create_conversation(agent, created_by).await
    }

    pub async fn create_conversation(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.has_agent(agent).await {
            bail!("agent '{agent}' not registered");
        }
        let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
        let conversation = Conversation::new(id, agent, created_by);
        self.conversations
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(conversation)));
        let mut index = self.conversation_index.write().await;
        Self::index_conversation(&mut index, id, agent, created_by);
        Ok(id)
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
        let mut conversation =
            Conversation::new(id, &snapshot.meta.agent, &snapshot.meta.created_by);
        conversation.history = self.resumed_history(snapshot.archive.as_deref(), snapshot.history);
        conversation.title = snapshot.meta.title;
        conversation.uptime_secs = snapshot.meta.uptime_secs;
        conversation.handle = Some(handle);
        self.conversations
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(conversation)));
        let mut index = self.conversation_index.write().await;
        Self::index_conversation(&mut index, id, &snapshot.meta.agent, &snapshot.meta.created_by);
        Ok(id)
    }

    pub async fn close_conversation(&self, id: u64) -> bool {
        self.steering.write().await.remove(&id);
        let removed = self.conversations.write().await.remove(&id);
        let Some(conversation_mutex) = removed else {
            return false;
        };

        let (agent, created_by) = {
            let conversation = conversation_mutex.lock().await;
            (conversation.agent.clone(), conversation.created_by.clone())
        };
        let key = Self::conversation_key(&agent, &created_by);

        let replacement = {
            let conversations = self.conversations.read().await;
            let mut replacement = None;
            for (candidate_id, candidate_mutex) in conversations.iter() {
                let candidate = candidate_mutex.lock().await;
                if candidate.agent == agent && candidate.created_by == created_by {
                    replacement = Some(*candidate_id);
                    break;
                }
            }
            replacement
        };

        let mut index = self.conversation_index.write().await;
        if index.get(&key).copied() == Some(id) {
            index.remove(&key);
            if let Some(replacement_id) = replacement {
                index.insert(key, replacement_id);
            }
        }

        true
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
        self.conversations.read().await.get(&id).cloned()
    }

    pub async fn conversations(&self) -> Vec<Arc<Mutex<Conversation>>> {
        self.conversations.read().await.values().cloned().collect()
    }

    pub async fn conversation_count(&self) -> usize {
        self.conversations.read().await.len()
    }

    pub async fn find_conversation_id(&self, agent: &str, sender: &str) -> Option<u64> {
        let key = Self::conversation_key(agent, sender);
        if let Some(id) = self.conversation_index.read().await.get(&key).copied() {
            return Some(id);
        }

        let id = self.find_conversation_id_slow(agent, sender).await?;
        let mut index = self.conversation_index.write().await;
        index.entry(key).or_insert(id);
        Some(id)
    }

    pub async fn compact_conversation(&self, conversation_id: u64) -> Option<String> {
        let (agent_name, history) = {
            let conversation_mutex = self
                .conversations
                .read()
                .await
                .get(&conversation_id)?
                .clone();
            let conversation = conversation_mutex.lock().await;
            if conversation.history.is_empty() {
                return None;
            }
            (conversation.agent.clone(), conversation.history.clone())
        };
        let persistent = self.agents.read().get(&agent_name).cloned();
        if let Some(a) = persistent {
            return a.compact(&history).await;
        }
        let a = self
            .ephemeral_agents
            .read()
            .await
            .get(&agent_name)
            .cloned()?;
        a.compact(&history).await
    }

    pub async fn transfer_conversations<C2: Config>(&self, dest: &mut Runtime<C2>) {
        let conversations = self.conversations.read().await;
        let dest_conversations = dest.conversations.get_mut();
        let dest_index = dest.conversation_index.get_mut();
        dest_index.clear();
        for (id, conversation) in conversations.iter() {
            dest_conversations.insert(*id, conversation.clone());
            let conversation = conversation.lock().await;
            Self::index_conversation(dest_index, *id, &conversation.agent, &conversation.created_by);
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

    /// Write a compaction summary to memory as an `Archive` entry and
    /// return the generated entry name. `None` on failure — the caller
    /// must skip the compact marker so a resume can't dangle.
    fn write_archive(&self, session_slug: &str, summary: String) -> Option<String> {
        let name = archive_base_name(session_slug);
        let mut mem = self.memory.write();
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
    /// the repo if needed. Called before the first persist.
    fn ensure_handle(&self, conversation: &mut Conversation) {
        if conversation.handle.is_some() {
            return;
        }
        let storage = self.storage();
        match storage.create_session(&conversation.agent, &conversation.created_by) {
            Ok(handle) => conversation.handle = Some(handle),
            Err(e) => tracing::warn!("failed to create session: {e}"),
        }
    }

    /// Persist messages to the session repo. Handles ensure_handle,
    /// compact markers, and meta updates.
    pub(crate) fn persist_messages(
        &self,
        conversation: &mut Conversation,
        pre_run_len: usize,
        compact_summary: Option<String>,
        event_trace: &[wcore::EventLine],
    ) {
        self.ensure_handle(conversation);
        let Some(ref handle) = conversation.handle else {
            return;
        };
        let storage = self.storage();

        if let Some(summary) = compact_summary {
            // Archive first — if this fails, don't write a dangling
            // marker that points at nothing.
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
        let _ = storage.update_session_meta(handle, &conversation.meta());
    }

    pub(crate) fn spawn_title_generation(
        &self,
        _conversation_id: u64,
        agent_name: &str,
        conversation_mutex: Arc<Mutex<Conversation>>,
    ) {
        let model = self.model.clone();
        let storage = self.storage().clone();
        let model_name = self
            .agents
            .read()
            .get(agent_name)
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
                                    let _ =
                                        storage.update_session_meta(handle, &conversation.meta());
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

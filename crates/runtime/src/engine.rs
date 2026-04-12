//! Runtime — agent registry, conversation management, and hook orchestration.
//!
//! [`Runtime`] holds agents as immutable definitions and conversations as
//! per-conversation `Arc<Mutex<Conversation>>` containers. Tool schemas and
//! handlers are registered by the caller at construction. Execution methods
//! (`send_to`, `stream_to`) take a conversation ID, lock the conversation,
//! clone the agent, and run with the conversation's history.

use crate::{Config, Conversation, Env, Hook};
use anyhow::{Result, bail};
use async_stream::stream;
use crabllm_core::{ChatCompletionRequest, Message, Role, ToolChoice};
use futures_core::Stream;
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, RwLock as StdRwLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex, RwLock, mpsc, watch};
use wcore::{
    Agent, AgentBuilder, AgentConfig, AgentEvent, AgentResponse, AgentStopReason, ToolDispatcher,
    ToolRegistry,
    model::{HistoryEntry, Model},
    storage::{SessionHandle, Storage},
};

/// The crabtalk runtime.
pub struct Runtime<C: Config> {
    pub model: Model<C::Provider>,
    pub hook: Arc<Env<C::Host>>,
    storage: Arc<C::Storage>,
    agents: StdRwLock<BTreeMap<String, Agent<C::Provider>>>,
    ephemeral_agents: RwLock<BTreeMap<String, Agent<C::Provider>>>,
    conversations: RwLock<BTreeMap<u64, Arc<Mutex<Conversation>>>>,
    next_conversation_id: AtomicU64,
    pub tools: ToolRegistry,
    steering: RwLock<BTreeMap<u64, watch::Sender<Option<String>>>>,
}

impl<C: Config> Runtime<C> {
    /// Create a new runtime with the given model, env, storage, and tools.
    pub fn new(
        model: Model<C::Provider>,
        hook: Env<C::Host>,
        storage: Arc<C::Storage>,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            model,
            hook: Arc::new(hook),
            storage,
            agents: StdRwLock::new(BTreeMap::new()),
            ephemeral_agents: RwLock::new(BTreeMap::new()),
            conversations: RwLock::new(BTreeMap::new()),
            next_conversation_id: AtomicU64::new(1),
            tools,
            steering: RwLock::new(BTreeMap::new()),
        }
    }

    /// Access the persistence backend.
    pub fn storage(&self) -> &Arc<C::Storage> {
        &self.storage
    }

    // --- Agent registry ---

    pub fn add_agent(&self, config: AgentConfig) {
        let _ = self.upsert_agent(config);
    }

    pub fn upsert_agent(&self, config: AgentConfig) -> AgentConfig {
        let (name, agent) = self.build_agent(config);
        let registered = agent.config.clone();
        self.agents
            .write()
            .expect("agents lock poisoned")
            .insert(name, agent);
        registered
    }

    pub fn remove_agent(&self, name: &str) -> bool {
        self.agents
            .write()
            .expect("agents lock poisoned")
            .remove(name)
            .is_some()
    }

    fn build_agent(&self, config: AgentConfig) -> (String, Agent<C::Provider>) {
        let config = self.hook.on_build_agent(config);
        let name = config.name.clone();
        let tools = self.tools.filtered_snapshot(&config.tools);
        let dispatcher: Arc<dyn ToolDispatcher> = self.hook.clone();
        let agent = AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools)
            .dispatcher(dispatcher)
            .build();
        (name, agent)
    }

    pub fn agent(&self, name: &str) -> Option<AgentConfig> {
        self.agents
            .read()
            .expect("agents lock poisoned")
            .get(name)
            .map(|a| a.config.clone())
    }

    pub fn agents(&self) -> Vec<AgentConfig> {
        self.agents
            .read()
            .expect("agents lock poisoned")
            .values()
            .map(|a| a.config.clone())
            .collect()
    }

    // --- Ephemeral agents ---

    pub async fn add_ephemeral(&self, config: AgentConfig) {
        let (name, agent) = self.build_agent(config);
        self.ephemeral_agents.write().await.insert(name, agent);
    }

    pub async fn remove_ephemeral(&self, name: &str) {
        self.ephemeral_agents.write().await.remove(name);
    }

    async fn resolve_agent(&self, name: &str) -> Option<Agent<C::Provider>> {
        let persistent = self
            .agents
            .read()
            .expect("agents lock poisoned")
            .get(name)
            .cloned();
        if persistent.is_some() {
            return persistent;
        }
        self.ephemeral_agents.read().await.get(name).cloned()
    }

    async fn has_agent(&self, name: &str) -> bool {
        let has_persistent = self
            .agents
            .read()
            .expect("agents lock poisoned")
            .contains_key(name);
        if has_persistent {
            return true;
        }
        self.ephemeral_agents.read().await.contains_key(name)
    }

    // --- Conversation management ---

    /// Get or create a conversation for the given (agent, created_by) identity.
    pub async fn get_or_create_conversation(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.has_agent(agent).await {
            bail!("agent '{agent}' not registered");
        }

        // 1. In-memory lookup.
        {
            let conversations = self.conversations.read().await;
            for (id, conversation_mutex) in conversations.iter() {
                let c = conversation_mutex.lock().await;
                if c.agent == agent && c.created_by == created_by {
                    return Ok(*id);
                }
            }
        }

        // 2. Storage lookup — find latest persisted session for this identity.
        let storage = self.storage();
        if let Ok(Some(handle)) = storage.find_latest_session(agent, created_by)
            && let Ok(Some(snapshot)) = storage.load_session(&handle)
        {
            let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
            let mut conversation = Conversation::new(id, agent, created_by);
            conversation.history = snapshot.history;
            conversation.title = snapshot.meta.title;
            conversation.uptime_secs = snapshot.meta.uptime_secs;
            conversation.handle = Some(handle);
            self.conversations
                .write()
                .await
                .insert(id, Arc::new(Mutex::new(conversation)));
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
        conversation.history = snapshot.history;
        conversation.title = snapshot.meta.title;
        conversation.uptime_secs = snapshot.meta.uptime_secs;
        conversation.handle = Some(handle);
        self.conversations
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(conversation)));
        Ok(id)
    }

    pub async fn close_conversation(&self, id: u64) -> bool {
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
        self.conversations.read().await.get(&id).cloned()
    }

    pub async fn conversations(&self) -> Vec<Arc<Mutex<Conversation>>> {
        self.conversations.read().await.values().cloned().collect()
    }

    pub async fn conversation_count(&self) -> usize {
        self.conversations.read().await.len()
    }

    pub async fn find_conversation_id(&self, agent: &str, sender: &str) -> Option<u64> {
        let conversations = self.conversations.read().await;
        for (id, conv_mutex) in conversations.iter() {
            let conv = conv_mutex.lock().await;
            if conv.agent == agent && conv.created_by == sender {
                return Some(*id);
            }
        }
        None
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
        let persistent = self
            .agents
            .read()
            .expect("agents lock poisoned")
            .get(&agent_name)
            .cloned();
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
        for (id, conversation) in conversations.iter() {
            dest_conversations.insert(*id, conversation.clone());
        }
        let next = self.next_conversation_id.load(Ordering::Relaxed);
        dest.next_conversation_id.store(next, Ordering::Relaxed);
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
    fn persist_messages(
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
            let _ = storage.append_session_compact(handle, &summary);
            if conversation.history.len() > 1 {
                let tail: Vec<_> = conversation.history[1..]
                    .iter()
                    .filter(|e| !e.auto_injected)
                    .cloned()
                    .collect();
                let _ = storage.append_session_messages(handle, &tail);
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

    fn spawn_title_generation(
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
            .expect("agents lock poisoned")
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

    // --- Execution ---

    fn prepare_history(
        &self,
        conversation: &mut Conversation,
        content: &str,
        sender: &str,
    ) -> String {
        let content = self
            .hook
            .preprocess(&conversation.agent, content)
            .unwrap_or_else(|| content.to_owned());
        if sender.is_empty() {
            conversation.history.push(HistoryEntry::user(&content));
        } else {
            conversation
                .history
                .push(HistoryEntry::user_with_sender(&content, sender));
        }

        conversation.history.retain(|e| !e.auto_injected);

        let agent_name = conversation.agent.clone();
        let recall_msgs =
            self.hook
                .on_before_run(&agent_name, conversation.id, &conversation.history);
        if !recall_msgs.is_empty() {
            let insert_pos = conversation.history.len().saturating_sub(1);
            for (i, entry) in recall_msgs.into_iter().enumerate() {
                conversation.history.insert(insert_pos + i, entry);
            }
        }
        agent_name
    }

    pub async fn send_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        tool_choice: Option<ToolChoice>,
    ) -> Result<AgentResponse> {
        let conversation_mutex = self
            .conversations
            .read()
            .await
            .get(&conversation_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("conversation {conversation_id} not found"))?;

        let mut conversation = conversation_mutex.lock().await;
        let pre_run_len = conversation.history.len();
        let agent_name = self.prepare_history(&mut conversation, content, sender);
        let agent = self
            .resolve_agent(&conversation.agent)
            .await
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", conversation.agent))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let run_start = std::time::Instant::now();
        let response = agent
            .run(&mut conversation.history, tx, None, tool_choice)
            .await;
        conversation.uptime_secs += run_start.elapsed().as_secs();

        let mut compact_summary: Option<String> = None;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Compact { ref summary } = event {
                compact_summary = Some(summary.clone());
            }
            self.hook.on_event(&agent_name, conversation_id, &event);
        }

        self.persist_messages(&mut conversation, pre_run_len, compact_summary, &[]);

        if conversation.title.is_empty() && conversation.history.len() >= 2 {
            self.spawn_title_generation(
                conversation_id,
                &conversation.agent,
                conversation_mutex.clone(),
            );
        }
        Ok(response)
    }

    pub fn stream_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        tool_choice: Option<ToolChoice>,
    ) -> impl Stream<Item = AgentEvent> + '_ {
        let content = content.to_owned();
        let sender = sender.to_owned();
        stream! {
            let Some(conversation_mutex) = self
                .conversations
                .read()
                .await
                .get(&conversation_id)
                .cloned()
            else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("conversation {conversation_id} not found"),
                ));
                return;
            };

            let mut conversation = conversation_mutex.lock().await;
            let pre_run_len = conversation.history.len();
            let agent_name = self.prepare_history(&mut conversation, &content, &sender);
            let Some(agent) = self.resolve_agent(&conversation.agent).await else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("agent '{}' not registered", conversation.agent),
                ));
                return;
            };

            let run_start = std::time::Instant::now();
            let (steer_tx, steer_rx) = watch::channel(None::<String>);
            self.steering.write().await.insert(conversation_id, steer_tx);
            let mut compact_summary: Option<String> = None;
            let mut done_event: Option<AgentEvent> = None;
            let mut event_trace: Vec<wcore::EventLine> = Vec::new();
            {
                let mut event_stream = std::pin::pin!(agent.run_stream(&mut conversation.history, Some(conversation_id), Some(steer_rx), tool_choice));
                while let Some(event) = event_stream.next().await {
                    if let AgentEvent::Compact { ref summary } = event {
                        compact_summary = Some(summary.clone());
                    }
                    self.hook.on_event(&agent_name, conversation_id, &event);
                    if let Some(line) = wcore::EventLine::from_agent_event(&event) {
                        event_trace.push(line);
                    }
                    if matches!(event, AgentEvent::Done(_)) {
                        done_event = Some(event);
                    } else {
                        yield event;
                    }
                }
            }
            self.steering.write().await.remove(&conversation_id);
            conversation.uptime_secs += run_start.elapsed().as_secs();
            self.persist_messages(&mut conversation, pre_run_len, compact_summary, &event_trace);

            if conversation.title.is_empty() && conversation.history.len() >= 2 {
                self.spawn_title_generation(conversation_id, &conversation.agent, conversation_mutex.clone());
            }
            if let Some(event) = done_event {
                yield event;
            }
        }
    }

    pub fn guest_stream_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
        guest: &str,
    ) -> impl Stream<Item = AgentEvent> + '_ {
        let content = content.to_owned();
        let sender = sender.to_owned();
        let guest = guest.to_owned();
        stream! {
            let Some(guest_agent) = self.resolve_agent(&guest).await else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("guest agent '{guest}' not registered"),
                ));
                return;
            };

            let Some(conversation_mutex) = self
                .conversations
                .read()
                .await
                .get(&conversation_id)
                .cloned()
            else {
                yield AgentEvent::Done(AgentResponse::error(
                    format!("conversation {conversation_id} not found"),
                ));
                return;
            };

            let mut conversation = conversation_mutex.lock().await;
            let pre_run_len = conversation.history.len();

            let content = self
                .hook
                .preprocess(&conversation.agent, &content)
                .unwrap_or_else(|| content.clone());
            if sender.is_empty() {
                conversation.history.push(HistoryEntry::user(&content));
            } else {
                conversation
                    .history
                    .push(HistoryEntry::user_with_sender(&content, &sender));
            }

            conversation.history.retain(|e| !e.auto_injected);

            let framing = HistoryEntry::system(format!(
                "You are joining a conversation as a guest. The primary agent is '{}'. \
                 Messages wrapped in <from agent=\"...\"> tags are from other agents. \
                 Respond as yourself to the user's latest message.",
                conversation.agent
            ))
            .auto_injected();
            let insert_pos = conversation.history.len().saturating_sub(1);
            conversation.history.insert(insert_pos, framing);

            let run_start = std::time::Instant::now();
            let model_name = guest_agent.config.model.clone().unwrap_or_default();

            let mut messages = Vec::with_capacity(1 + conversation.history.len());
            if !guest_agent.config.system_prompt.is_empty() {
                messages.push(Message::system(&guest_agent.config.system_prompt));
            }
            messages.extend(conversation.history.iter().map(|e| e.to_wire_message()));

            let request = ChatCompletionRequest {
                model: model_name.clone(),
                messages,
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
                reasoning_effort: if guest_agent.config.thinking {
                    Some("high".to_string())
                } else {
                    None
                },
                extra: Default::default(),
            };

            let mut response_text = String::new();
            let mut reasoning = String::new();
            {
                let mut stream = std::pin::pin!(self.model.stream_ct(request));
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(chunk) => {
                            if let Some(text) = chunk.content() {
                                response_text.push_str(text);
                                yield AgentEvent::TextDelta(text.to_string());
                            }
                            if let Some(text) = chunk.reasoning_content() {
                                reasoning.push_str(text);
                                yield AgentEvent::ThinkingDelta(text.to_string());
                            }
                        }
                        Err(e) => {
                            yield AgentEvent::Done(AgentResponse {
                                final_response: None,
                                iterations: 1,
                                stop_reason: AgentStopReason::Error(e.to_string()),
                                steps: vec![],
                                model: model_name.clone(),
                            });
                            return;
                        }
                    }
                }
            }

            let reasoning = if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            };
            let mut response_entry = HistoryEntry::assistant(&response_text, reasoning, None);
            response_entry.agent = guest.clone();
            conversation.history.push(response_entry);

            conversation.uptime_secs += run_start.elapsed().as_secs();
            self.persist_messages(&mut conversation, pre_run_len, None, &[]);

            if conversation.title.is_empty() && conversation.history.len() >= 2 {
                self.spawn_title_generation(conversation_id, &conversation.agent, conversation_mutex.clone());
            }

            yield AgentEvent::Done(AgentResponse {
                final_response: Some(response_text),
                iterations: 1,
                stop_reason: AgentStopReason::TextResponse,
                steps: vec![],
                model: model_name,
            });
        }
    }
}

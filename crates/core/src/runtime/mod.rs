//! Runtime — agent registry, conversation management, and hook orchestration.
//!
//! [`Runtime`] holds agents as immutable definitions and conversations as
//! per-conversation `Arc<Mutex<Conversation>>` containers. Tool schemas are registered
//! once at startup via `hook.on_register_tools()`. Execution methods
//! (`send_to`, `stream_to`) take a conversation ID, lock the conversation, clone the
//! agent, and run with the conversation's history.

use crate::{
    Agent, AgentBuilder, AgentConfig, AgentEvent, AgentResponse, AgentStopReason,
    agent::tool::{ToolRegistry, ToolSender},
    model::{HistoryEntry, Model},
    runtime::hook::Hook,
};
use anyhow::{Result, bail};
use async_stream::stream;
use crabllm_core::{ChatCompletionRequest, Message, Provider, Role, ToolChoice};
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

pub mod conversation;
pub mod hook;

pub use conversation::Conversation;

/// The crabtalk runtime — agent registry, conversation store, and hook orchestration.
///
/// Generic over `P: crabllm_core::Provider` for the LLM backend, and
/// `H: Hook` for the runtime environment. Agents are stored as plain
/// immutable values. Conversations own conversation history behind
/// per-conversation `Arc<Mutex<Conversation>>`. The conversations map
/// uses `RwLock` for concurrent access without requiring `&mut self`.
pub struct Runtime<P: Provider + 'static, H: Hook> {
    pub model: Model<P>,
    pub hook: H,
    /// Registered persistent agents, behind a sync `RwLock` so CRUD can
    /// mutate in place without requiring `&mut Runtime`. Critical sections
    /// are short (lookup, insert, remove) and never hold the guard across
    /// an `.await`.
    agents: StdRwLock<BTreeMap<String, Agent<P>>>,
    /// Ephemeral agents created by delegate, invisible to client APIs.
    ephemeral_agents: RwLock<BTreeMap<String, Agent<P>>>,
    conversations: RwLock<BTreeMap<u64, Arc<Mutex<Conversation>>>>,
    next_conversation_id: AtomicU64,
    pub tools: ToolRegistry,
    tool_tx: Option<ToolSender>,
    /// Per-conversation steering senders, active only while a stream is running.
    steering: RwLock<BTreeMap<u64, watch::Sender<Option<String>>>>,
}

impl<P: Provider + 'static, H: Hook + 'static> Runtime<P, H> {
    /// Create a new runtime with the given model and hook backend.
    ///
    /// Calls `hook.on_register_tools()` to populate the schema registry.
    /// Pass `tool_tx` to enable tool dispatch from agents; `None` means agents
    /// have no tool dispatch (e.g. CLI without a daemon).
    pub async fn new(model: Model<P>, hook: H, tool_tx: Option<ToolSender>) -> Self {
        let mut tools = ToolRegistry::new();
        hook.on_register_tools(&mut tools).await;
        Self {
            model,
            hook,
            agents: StdRwLock::new(BTreeMap::new()),
            ephemeral_agents: RwLock::new(BTreeMap::new()),
            conversations: RwLock::new(BTreeMap::new()),
            next_conversation_id: AtomicU64::new(1),
            tools,
            tool_tx,
            steering: RwLock::new(BTreeMap::new()),
        }
    }

    // --- Agent registry ---

    /// Register an agent from its configuration.
    ///
    /// Calls `hook.on_build_agent(config)` to enrich the config, then builds
    /// the agent with a filtered schema snapshot and the runtime's `tool_tx`.
    /// If an agent with the same name already exists it is replaced.
    pub fn add_agent(&self, config: AgentConfig) {
        let _ = self.upsert_agent(config);
    }

    /// Insert-or-replace an agent and return the post-`on_build_agent`
    /// config — the form actually stored in the registry. CRUD call sites
    /// use the returned config to register a matching dispatch scope
    /// without a second lookup (and without a TOCTOU window against a
    /// concurrent `remove_agent`).
    pub fn upsert_agent(&self, config: AgentConfig) -> AgentConfig {
        let (name, agent) = self.build_agent(config);
        let registered = agent.config.clone();
        self.agents
            .write()
            .expect("agents lock poisoned")
            .insert(name, agent);
        registered
    }

    /// Remove a persistent agent by name. Returns `true` if an entry was
    /// removed. Ephemeral agents are not affected.
    pub fn remove_agent(&self, name: &str) -> bool {
        self.agents
            .write()
            .expect("agents lock poisoned")
            .remove(name)
            .is_some()
    }

    /// Build an agent from config. Returns (name, agent).
    fn build_agent(&self, config: AgentConfig) -> (String, Agent<P>) {
        let config = self.hook.on_build_agent(config);
        let name = config.name.clone();
        let tools = self.tools.filtered_snapshot(&config.tools);
        let mut builder = AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools);
        if let Some(tx) = &self.tool_tx {
            builder = builder.tool_tx(tx.clone());
        }
        (name, builder.build())
    }

    /// Get a registered agent's config by name (cloned).
    pub fn agent(&self, name: &str) -> Option<AgentConfig> {
        self.agents
            .read()
            .expect("agents lock poisoned")
            .get(name)
            .map(|a| a.config.clone())
    }

    /// Get all registered agent configs (cloned, alphabetical order).
    pub fn agents(&self) -> Vec<AgentConfig> {
        self.agents
            .read()
            .expect("agents lock poisoned")
            .values()
            .map(|a| a.config.clone())
            .collect()
    }

    // --- Ephemeral agents ---

    /// Register an ephemeral agent (delegate-created, invisible to clients).
    pub async fn add_ephemeral(&self, config: AgentConfig) {
        let (name, agent) = self.build_agent(config);
        self.ephemeral_agents.write().await.insert(name, agent);
    }

    /// Remove an ephemeral agent by name.
    pub async fn remove_ephemeral(&self, name: &str) {
        self.ephemeral_agents.write().await.remove(name);
    }

    /// Resolve an agent by name: persistent first, then ephemeral. Returns a
    /// clone so no lock is held during the (potentially long) agent run.
    async fn resolve_agent(&self, name: &str) -> Option<Agent<P>> {
        // Snapshot into a local so the sync guard is dropped before any
        // `.await` below — read guards aren't `Send`.
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

    /// Check if an agent exists in either the persistent or ephemeral store.
    async fn has_agent(&self, name: &str) -> bool {
        // Scope the sync guard so it drops before the ephemeral `.await`.
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
    ///
    /// 1. Check in-memory conversations for a match -> return existing ID.
    /// 2. Check disk for a persisted conversation file -> load context, return ID.
    /// 3. Neither -> create a new conversation with a fresh file.
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

        // 2. Disk lookup — find latest conversation file for this identity.
        if let Some(path) = conversation::find_latest_conversation(
            &crate::paths::CONVERSATIONS_DIR,
            agent,
            created_by,
        ) && let Ok((meta, messages)) = Conversation::load_context(&path)
        {
            let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
            let mut conversation = Conversation::new(id, agent, created_by);
            conversation.history = messages;
            conversation.title = meta.title;
            conversation.uptime_secs = meta.uptime_secs;
            conversation.file_path = Some(path);
            self.conversations
                .write()
                .await
                .insert(id, Arc::new(Mutex::new(conversation)));
            return Ok(id);
        }

        // 3. Create new.
        self.create_conversation(agent, created_by).await
    }

    /// Create a new conversation for the given agent. Returns the conversation ID.
    ///
    /// The JSONL file is not created here — it is deferred until the first
    /// message is persisted via [`Conversation::ensure_file`], avoiding ghost
    /// conversation files from connections that drop before any exchange.
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

    /// Load a specific conversation from a file path. Returns the conversation ID.
    pub async fn load_specific_conversation(&self, file_path: &std::path::Path) -> Result<u64> {
        let (meta, messages) = Conversation::load_context(file_path)?;
        if !self.has_agent(&meta.agent).await {
            bail!("agent '{}' not registered", meta.agent);
        }
        let id = self.next_conversation_id.fetch_add(1, Ordering::Relaxed);
        let mut conversation = Conversation::new(id, &meta.agent, &meta.created_by);
        conversation.history = messages;
        conversation.title = meta.title;
        conversation.uptime_secs = meta.uptime_secs;
        conversation.file_path = Some(file_path.to_path_buf());
        self.conversations
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(conversation)));
        Ok(id)
    }

    /// Close (remove) a conversation by ID. Returns true if it existed.
    pub async fn close_conversation(&self, id: u64) -> bool {
        self.steering.write().await.remove(&id);
        self.conversations.write().await.remove(&id).is_some()
    }

    /// Send a steering message to an active stream.
    ///
    /// The message will be injected into the agent's history at the next turn
    /// boundary (after the current tool dispatch completes, before the next
    /// model call). Returns an error if no stream is active for the conversation.
    pub async fn steer(&self, conversation_id: u64, content: String) -> Result<()> {
        let senders = self.steering.read().await;
        let tx = senders.get(&conversation_id).ok_or_else(|| {
            anyhow::anyhow!("no active stream for conversation {conversation_id}")
        })?;
        tx.send(Some(content))
            .map_err(|_| anyhow::anyhow!("steering channel closed"))?;
        Ok(())
    }

    /// Get a conversation mutex by ID.
    pub async fn conversation(&self, id: u64) -> Option<Arc<Mutex<Conversation>>> {
        self.conversations.read().await.get(&id).cloned()
    }

    /// Get all conversation mutexes (for iteration/listing).
    pub async fn conversations(&self) -> Vec<Arc<Mutex<Conversation>>> {
        self.conversations.read().await.values().cloned().collect()
    }

    /// Number of open conversations (created and not yet killed).
    pub async fn conversation_count(&self) -> usize {
        self.conversations.read().await.len()
    }

    /// Find the internal conversation ID for a given (agent, sender) identity.
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

    /// Compact a conversation's history into a concise summary.
    ///
    /// Clones history to release the lock before the LLM call.
    /// Returns `None` if conversation/agent not found, history empty, or LLM fails.
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
        // Snapshot first so the sync read guard drops before any `.await`
        // below — read guards aren't `Send`.
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

    /// Move all conversations from this runtime into `dest`.
    ///
    /// Used during daemon reload to preserve gateway conversations. The `dest`
    /// runtime must not yet be shared (call before wrapping in `Arc`).
    pub async fn transfer_conversations<P2: Provider + 'static, H2: Hook>(
        &self,
        dest: &mut Runtime<P2, H2>,
    ) {
        let conversations = self.conversations.read().await;
        let dest_conversations = dest.conversations.get_mut();
        for (id, conversation) in conversations.iter() {
            dest_conversations.insert(*id, conversation.clone());
        }
        let next = self.next_conversation_id.load(Ordering::Relaxed);
        dest.next_conversation_id.store(next, Ordering::Relaxed);
    }

    /// Spawn a background task to generate a conversation title from the
    /// first user+assistant exchange. Non-blocking — the main flow continues.
    ///
    /// `agent_name` is the conversation's agent — its `config.model` is used
    /// for the title-generation request. Title gen runs on the same model
    /// the agent uses, since the agent's model is the only entity in the
    /// runtime with an opinion about which model fits this conversation.
    fn spawn_title_generation(
        &self,
        _conversation_id: u64,
        agent_name: &str,
        conversation_mutex: Arc<Mutex<Conversation>>,
    ) {
        let model = self.model.clone();
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

            // Truncate to keep the title-generation request small.
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
                                conversation.set_title(&title);
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

    /// Push the user message, strip old auto-injected messages, and inject
    /// fresh ones via `on_before_run`. Returns the agent name.
    fn prepare_history(
        &self,
        conversation: &mut Conversation,
        content: &str,
        sender: &str,
    ) -> String {
        let content = self.hook.preprocess(&conversation.agent, content);
        if sender.is_empty() {
            conversation.history.push(HistoryEntry::user(&content));
        } else {
            conversation
                .history
                .push(HistoryEntry::user_with_sender(&content, sender));
        }

        // Strip previous auto-injected entries to avoid accumulation.
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

    /// Send a message to a conversation and run to completion.
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

        // Drain events, stash compact summary if one occurred.
        let mut compact_summary: Option<String> = None;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Compact { ref summary } = event {
                compact_summary = Some(summary.clone());
            }
            self.hook.on_event(&agent_name, conversation_id, &event);
        }

        // Create the JSONL file on first persist (deferred from create_conversation).
        conversation.ensure_file();

        // Append-only persistence.
        if let Some(summary) = compact_summary {
            // Compaction happened: append compact marker + post-compact messages.
            conversation.append_compact(&summary);
            // history[0] is the summary-as-user-message; skip it (compact line serves that role).
            if conversation.history.len() > 1 {
                conversation.append_messages(&conversation.history[1..]);
            }
        } else {
            // No compaction: append new messages since pre_run.
            conversation.append_messages(&conversation.history[pre_run_len..]);
        }

        // Persist updated uptime to meta line.
        conversation.rewrite_meta();

        // Generate title in background if this is the first exchange.
        if conversation.title.is_empty() && conversation.history.len() >= 2 {
            self.spawn_title_generation(
                conversation_id,
                &conversation.agent,
                conversation_mutex.clone(),
            );
        }
        Ok(response)
    }

    /// Send a message to a conversation and stream response events.
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
            let mut event_trace: Vec<crate::runtime::conversation::EventLine> = Vec::new();
            {
                let mut event_stream = std::pin::pin!(agent.run_stream(&mut conversation.history, Some(conversation_id), Some(steer_rx), tool_choice));
                while let Some(event) = event_stream.next().await {
                    if let AgentEvent::Compact { ref summary } = event {
                        compact_summary = Some(summary.clone());
                    }
                    self.hook.on_event(&agent_name, conversation_id, &event);
                    if let Some(line) = crate::runtime::conversation::EventLine::from_agent_event(&event) {
                        event_trace.push(line);
                    }
                    // Hold back Done — yield it after persistence.
                    if matches!(event, AgentEvent::Done(_)) {
                        done_event = Some(event);
                    } else {
                        yield event;
                    }
                }
            }
            // Borrow on conversation.history is released. Clean up steering and persist.
            self.steering.write().await.remove(&conversation_id);
            conversation.uptime_secs += run_start.elapsed().as_secs();
            // Create the JSONL file on first persist (deferred from create_conversation).
            conversation.ensure_file();
            if let Some(summary) = compact_summary {
                conversation.append_compact(&summary);
                if conversation.history.len() > 1 {
                    conversation.append_messages(&conversation.history[1..]);
                }
            } else {
                conversation.append_messages(&conversation.history[pre_run_len..]);
            }
            conversation.append_events(&event_trace);
            // Persist updated uptime to meta line.
            conversation.rewrite_meta();

            // Generate title in background if this is the first exchange.
            if conversation.title.is_empty() && conversation.history.len() >= 2 {
                self.spawn_title_generation(conversation_id, &conversation.agent, conversation_mutex.clone());
            }
            // Now yield Done.
            if let Some(event) = done_event {
                yield event;
            }
        }
    }

    /// Run a guest agent against a conversation's history for a single turn.
    ///
    /// The user message is added to the primary agent's conversation. The guest
    /// agent responds with its own system prompt but no tools (v1: advisors,
    /// not operators). The response is tagged with the guest's name and
    /// appended to the primary's history.
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
            // Validate guest agent exists.
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

            // Add user message to the primary's history.
            let content = self.hook.preprocess(&conversation.agent, &content);
            if sender.is_empty() {
                conversation.history.push(HistoryEntry::user(&content));
            } else {
                conversation
                    .history
                    .push(HistoryEntry::user_with_sender(&content, &sender));
            }

            // Strip old auto-injected entries.
            conversation.history.retain(|e| !e.auto_injected);

            // Inject guest framing as auto_injected so it gets stripped next run.
            let framing = HistoryEntry::system(format!(
                "You are joining a conversation as a guest. The primary agent is '{}'. \
                 Messages wrapped in <from agent=\"...\"> tags are from other agents. \
                 Respond as yourself to the user's latest message.",
                conversation.agent
            ))
            .auto_injected();
            let insert_pos = conversation.history.len().saturating_sub(1);
            conversation.history.insert(insert_pos, framing);

            // Run the guest agent — text-only, no tools. The guest is an
            // advisor: it reads the conversation and responds, but cannot
            // execute tools, call APIs, or mutate files.
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

            // Stream the response token-by-token — text only, no tool dispatch.
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

            // Append the guest's response to the conversation history, tagged.
            let reasoning = if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            };
            let mut response_entry = HistoryEntry::assistant(&response_text, reasoning, None);
            response_entry.agent = guest.clone();
            conversation.history.push(response_entry);

            // Persist.
            conversation.uptime_secs += run_start.elapsed().as_secs();
            conversation.ensure_file();
            conversation.append_messages(&conversation.history[pre_run_len..]);
            conversation.rewrite_meta();

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

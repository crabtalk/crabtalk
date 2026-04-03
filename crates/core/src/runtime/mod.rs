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
    model::{Message, Model},
    runtime::hook::Hook,
};
use anyhow::{Result, bail};
use async_stream::stream;
use futures_core::Stream;
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex, RwLock, mpsc};

pub mod conversation;
pub mod hook;

pub use conversation::Conversation;

/// The crabtalk runtime — agent registry, conversation store, and hook orchestration.
///
/// Agents are stored as plain immutable values. Conversations own conversation
/// history behind per-conversation `Arc<Mutex<Conversation>>`. The conversations map uses
/// `RwLock` for concurrent access without requiring `&mut self`.
pub struct Runtime<M: Model, H: Hook> {
    pub model: M,
    pub hook: H,
    agents: BTreeMap<String, Agent<M>>,
    conversations: RwLock<BTreeMap<u64, Arc<Mutex<Conversation>>>>,
    next_conversation_id: AtomicU64,
    pub tools: ToolRegistry,
    tool_tx: Option<ToolSender>,
}

impl<M: Model + Send + Sync + Clone + 'static, H: Hook + 'static> Runtime<M, H> {
    /// Create a new runtime with the given model and hook backend.
    ///
    /// Calls `hook.on_register_tools()` to populate the schema registry.
    /// Pass `tool_tx` to enable tool dispatch from agents; `None` means agents
    /// have no tool dispatch (e.g. CLI without a daemon).
    pub async fn new(model: M, hook: H, tool_tx: Option<ToolSender>) -> Self {
        let mut tools = ToolRegistry::new();
        hook.on_register_tools(&mut tools).await;
        Self {
            model,
            hook,
            agents: BTreeMap::new(),
            conversations: RwLock::new(BTreeMap::new()),
            next_conversation_id: AtomicU64::new(1),
            tools,
            tool_tx,
        }
    }

    // --- Agent registry ---

    /// Register an agent from its configuration.
    ///
    /// Calls `hook.on_build_agent(config)` to enrich the config, then builds
    /// the agent with a filtered schema snapshot and the runtime's `tool_tx`.
    pub fn add_agent(&mut self, config: AgentConfig) {
        let config = self.hook.on_build_agent(config);
        let name = config.name.clone();
        let tools = self.tools.filtered_snapshot(&config.tools);
        let mut builder = AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools);
        if let Some(tx) = &self.tool_tx {
            builder = builder.tool_tx(tx.clone());
        }
        let agent = builder.build();
        self.agents.insert(name, agent);
    }

    /// Get a registered agent's config by name (cloned).
    pub fn agent(&self, name: &str) -> Option<AgentConfig> {
        self.agents.get(name).map(|a| a.config.clone())
    }

    /// Get all registered agent configs (cloned, alphabetical order).
    pub fn agents(&self) -> Vec<AgentConfig> {
        self.agents.values().map(|a| a.config.clone()).collect()
    }

    /// Get a reference to an agent by name.
    pub fn get_agent(&self, name: &str) -> Option<&Agent<M>> {
        self.agents.get(name)
    }

    // --- Conversation management ---

    /// Get or create a conversation for the given (agent, created_by) identity.
    ///
    /// 1. Check in-memory conversations for a match -> return existing ID.
    /// 2. Check disk for a persisted conversation file -> load context, return ID.
    /// 3. Neither -> create a new conversation with a fresh file.
    pub async fn get_or_create_conversation(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.agents.contains_key(agent) {
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
        if !self.agents.contains_key(agent) {
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
        if !self.agents.contains_key(&meta.agent) {
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
        self.conversations.write().await.remove(&id).is_some()
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
        self.agents.get(&agent_name)?.compact(&history).await
    }

    /// Move all conversations from this runtime into `dest`.
    ///
    /// Used during daemon reload to preserve gateway conversations. The `dest`
    /// runtime must not yet be shared (call before wrapping in `Arc`).
    pub async fn transfer_conversations<M2: Model, H2: Hook>(&self, dest: &mut Runtime<M2, H2>) {
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
    fn spawn_title_generation(
        &self,
        _conversation_id: u64,
        conversation_mutex: Arc<Mutex<Conversation>>,
    ) {
        let model = self.model.clone();
        tokio::spawn(async move {
            let (user_msg, assistant_msg) = {
                let conversation = conversation_mutex.lock().await;
                let user = conversation
                    .history
                    .iter()
                    .find(|m| m.role == crate::model::Role::User && !m.auto_injected)
                    .map(|m| m.content.clone());
                let assistant = conversation
                    .history
                    .iter()
                    .find(|m| m.role == crate::model::Role::Assistant)
                    .map(|m| m.content.clone());
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

            let request = crate::model::Request::new(model.active_model())
                .with_messages(vec![Message::user(&prompt)]);

            match model.send(&request).await {
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
            conversation.history.push(Message::user(&content));
        } else {
            conversation
                .history
                .push(Message::user_with_sender(&content, sender));
        }

        // Strip previous auto-injected messages to avoid accumulation.
        conversation.history.retain(|m| !m.auto_injected);

        let agent_name = conversation.agent.clone();
        let recall_msgs =
            self.hook
                .on_before_run(&agent_name, conversation.id, &conversation.history);
        if !recall_msgs.is_empty() {
            let insert_pos = conversation.history.len().saturating_sub(1);
            for (i, msg) in recall_msgs.into_iter().enumerate() {
                conversation.history.insert(insert_pos + i, msg);
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
        let agent_ref = self
            .agents
            .get(&conversation.agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", conversation.agent))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let run_start = std::time::Instant::now();
        let response = agent_ref.run(&mut conversation.history, tx, None).await;
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
            self.spawn_title_generation(conversation_id, conversation_mutex.clone());
        }
        Ok(response)
    }

    /// Send a message to a conversation and stream response events.
    pub fn stream_to(
        &self,
        conversation_id: u64,
        content: &str,
        sender: &str,
    ) -> impl Stream<Item = AgentEvent> + '_ {
        let content = content.to_owned();
        let sender = sender.to_owned();
        stream! {
            let conversation_mutex = match self
                .conversations
                .read()
                .await
                .get(&conversation_id)
                .cloned()
            {
                Some(m) => m,
                None => {
                    let resp = AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("conversation {conversation_id} not found"),
                        ),
                        steps: vec![],
                        // No model involved in pre-run errors.
                        model: String::new(),
                    };
                    yield AgentEvent::Done(resp);
                    return;
                }
            };

            let mut conversation = conversation_mutex.lock().await;
            let pre_run_len = conversation.history.len();
            let agent_name = self.prepare_history(&mut conversation, &content, &sender);
            let agent_ref = match self.agents.get(&conversation.agent) {
                Some(a) => a,
                None => {
                    let resp = AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("agent '{}' not registered", conversation.agent),
                        ),
                        steps: vec![],
                        // No model involved in pre-run errors.
                        model: String::new(),
                    };
                    yield AgentEvent::Done(resp);
                    return;
                }
            };

            let run_start = std::time::Instant::now();
            let mut compact_summary: Option<String> = None;
            let mut done_event: Option<AgentEvent> = None;
            {
                let mut event_stream = std::pin::pin!(agent_ref.run_stream(&mut conversation.history, Some(conversation_id)));
                while let Some(event) = event_stream.next().await {
                    if let AgentEvent::Compact { ref summary } = event {
                        compact_summary = Some(summary.clone());
                    }
                    self.hook.on_event(&agent_name, conversation_id, &event);
                    // Hold back Done — yield it after persistence.
                    if matches!(event, AgentEvent::Done(_)) {
                        done_event = Some(event);
                    } else {
                        yield event;
                    }
                }
            }
            // Borrow on conversation.history is released. Persist now.
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
            // Persist updated uptime to meta line.
            conversation.rewrite_meta();

            // Generate title in background if this is the first exchange.
            if conversation.title.is_empty() && conversation.history.len() >= 2 {
                self.spawn_title_generation(conversation_id, conversation_mutex.clone());
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
            let guest_agent = match self.agents.get(&guest) {
                Some(a) => a,
                None => {
                    yield AgentEvent::Done(AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("guest agent '{guest}' not registered"),
                        ),
                        steps: vec![],
                        model: String::new(),
                    });
                    return;
                }
            };

            let conversation_mutex = match self
                .conversations
                .read()
                .await
                .get(&conversation_id)
                .cloned()
            {
                Some(m) => m,
                None => {
                    yield AgentEvent::Done(AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("conversation {conversation_id} not found"),
                        ),
                        steps: vec![],
                        model: String::new(),
                    });
                    return;
                }
            };

            let mut conversation = conversation_mutex.lock().await;
            let pre_run_len = conversation.history.len();

            // Add user message to the primary's history.
            let content = self.hook.preprocess(&conversation.agent, &content);
            if sender.is_empty() {
                conversation.history.push(Message::user(&content));
            } else {
                conversation
                    .history
                    .push(Message::user_with_sender(&content, &sender));
            }

            // Strip old auto-injected messages.
            conversation.history.retain(|m| !m.auto_injected);

            // Inject guest framing as auto_injected so it gets stripped next run.
            let mut framing = Message::system(format!(
                "You are joining a conversation as a guest. The primary agent is '{}'. \
                 Messages prefixed with [agent_name] are from other agents. \
                 Respond as yourself to the user's latest message.",
                conversation.agent
            ));
            framing.auto_injected = true;
            let insert_pos = conversation.history.len().saturating_sub(1);
            conversation.history.insert(insert_pos, framing);

            // Run the guest agent — text-only, no tools. The guest is an
            // advisor: it reads the conversation and responds, but cannot
            // execute tools, call APIs, or mutate files.
            let run_start = std::time::Instant::now();
            let model_name = guest_agent
                .config
                .model
                .clone()
                .unwrap_or_else(|| self.model.active_model());

            let mut messages = Vec::with_capacity(1 + conversation.history.len());
            if !guest_agent.config.system_prompt.is_empty() {
                messages.push(Message::system(&guest_agent.config.system_prompt));
            }
            messages.extend(conversation.history.iter().map(|m| m.with_agent_prefix()));

            let request = crate::model::Request::new(model_name.clone())
                .with_messages(messages)
                .with_think(guest_agent.config.thinking);

            // Stream the response token-by-token — text only, no tool dispatch.
            let mut response_content = String::new();
            let mut reasoning = String::new();
            {
                let mut stream = std::pin::pin!(self.model.stream(request));
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(chunk) => {
                            if let Some(text) = chunk.content() {
                                response_content.push_str(text);
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
            let mut response_msg = Message::assistant(&response_content, reasoning, None);
            response_msg.agent = guest.clone();
            conversation.history.push(response_msg);

            // Persist.
            conversation.uptime_secs += run_start.elapsed().as_secs();
            conversation.ensure_file();
            conversation.append_messages(&conversation.history[pre_run_len..]);
            conversation.rewrite_meta();

            if conversation.title.is_empty() && conversation.history.len() >= 2 {
                self.spawn_title_generation(conversation_id, conversation_mutex.clone());
            }

            yield AgentEvent::Done(AgentResponse {
                final_response: Some(response_content),
                iterations: 1,
                stop_reason: AgentStopReason::TextResponse,
                steps: vec![],
                model: model_name,
            });
        }
    }
}

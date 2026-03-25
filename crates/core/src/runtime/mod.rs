//! Runtime — agent registry, session management, and hook orchestration.
//!
//! [`Runtime`] holds agents as immutable definitions and sessions as
//! per-session `Arc<Mutex<Session>>` containers. Tool schemas are registered
//! once at startup via `hook.on_register_tools()`. Execution methods
//! (`send_to`, `stream_to`) take a session ID, lock the session, clone the
//! agent, and run with the session's history.

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
    collections::{BTreeMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex, RwLock, mpsc};

pub mod hook;
pub mod session;

pub use session::Session;

/// The crabtalk runtime — agent registry, session store, and hook orchestration.
///
/// Agents are stored as plain immutable values. Sessions own conversation
/// history behind per-session `Arc<Mutex<Session>>`. The sessions map uses
/// `RwLock` for concurrent access without requiring `&mut self`.
pub struct Runtime<M: Model, H: Hook> {
    pub model: M,
    pub hook: H,
    agents: BTreeMap<String, Agent<M>>,
    sessions: RwLock<BTreeMap<u64, Arc<Mutex<Session>>>>,
    next_session_id: AtomicU64,
    tools: ToolRegistry,
    tool_tx: Option<ToolSender>,
    active_sessions: RwLock<HashSet<u64>>,
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
            sessions: RwLock::new(BTreeMap::new()),
            next_session_id: AtomicU64::new(1),
            tools,
            tool_tx,
            active_sessions: RwLock::new(HashSet::new()),
        }
    }

    // --- Tool registry ---

    /// Register a tool schema.
    pub fn register_tool(&mut self, tool: crate::model::Tool) {
        self.tools.insert(tool);
    }

    /// Remove a tool schema by name. Returns `true` if it existed.
    pub fn unregister_tool(&mut self, name: &str) -> bool {
        self.tools.remove(name)
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

    // --- Session management ---

    /// Get or create a session for the given (agent, created_by) identity.
    ///
    /// 1. Check in-memory sessions for a match → return existing ID.
    /// 2. Check disk for a persisted session file → load context, return ID.
    /// 3. Neither → create a new session with a fresh file.
    pub async fn get_or_create_session(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.agents.contains_key(agent) {
            bail!("agent '{agent}' not registered");
        }

        // 1. In-memory lookup.
        {
            let sessions = self.sessions.read().await;
            for (id, session_mutex) in sessions.iter() {
                let s = session_mutex.lock().await;
                if s.agent == agent && s.created_by == created_by {
                    return Ok(*id);
                }
            }
        }

        // 2. Disk lookup — find latest session file for this identity.
        if let Some(path) =
            session::find_latest_session(&crate::paths::SESSIONS_DIR, agent, created_by)
            && let Ok((meta, messages)) = Session::load_context(&path)
        {
            let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
            let mut session = Session::new(id, agent, created_by);
            session.history = messages;
            session.title = meta.title;
            session.uptime_secs = meta.uptime_secs;
            session.file_path = Some(path);
            self.sessions
                .write()
                .await
                .insert(id, Arc::new(Mutex::new(session)));
            return Ok(id);
        }

        // 3. Create new.
        self.create_session(agent, created_by).await
    }

    /// Create a new session for the given agent. Returns the session ID.
    pub async fn create_session(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.agents.contains_key(agent) {
            bail!("agent '{agent}' not registered");
        }
        let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let mut session = Session::new(id, agent, created_by);
        session.init_file(&crate::paths::SESSIONS_DIR);
        self.sessions
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(session)));
        Ok(id)
    }

    /// Close (remove) a session by ID. Returns true if it existed.
    pub async fn close_session(&self, id: u64) -> bool {
        self.sessions.write().await.remove(&id).is_some()
    }

    /// Get a session mutex by ID.
    pub async fn session(&self, id: u64) -> Option<Arc<Mutex<Session>>> {
        self.sessions.read().await.get(&id).cloned()
    }

    /// Get all session mutexes (for iteration/listing).
    pub async fn sessions(&self) -> Vec<Arc<Mutex<Session>>> {
        self.sessions.read().await.values().cloned().collect()
    }

    /// Check if a session is currently active (running send_to or stream_to).
    pub async fn is_active(&self, id: u64) -> bool {
        self.active_sessions.read().await.contains(&id)
    }

    /// Move all sessions from this runtime into `dest`.
    ///
    /// Used during daemon reload to preserve gateway sessions. The `dest`
    /// runtime must not yet be shared (call before wrapping in `Arc`).
    pub async fn transfer_sessions<M2: Model, H2: Hook>(&self, dest: &mut Runtime<M2, H2>) {
        let sessions = self.sessions.read().await;
        let dest_sessions = dest.sessions.get_mut();
        for (id, session) in sessions.iter() {
            dest_sessions.insert(*id, session.clone());
        }
        let next = self.next_session_id.load(Ordering::Relaxed);
        dest.next_session_id.store(next, Ordering::Relaxed);
    }

    /// Spawn a background task to generate a conversation title from the
    /// first user+assistant exchange. Non-blocking — the main flow continues.
    fn spawn_title_generation(&self, _session_id: u64, session_mutex: Arc<Mutex<Session>>) {
        let model = self.model.clone();
        tokio::spawn(async move {
            let (user_msg, assistant_msg) = {
                let session = session_mutex.lock().await;
                let user = session
                    .history
                    .iter()
                    .find(|m| m.role == crate::model::Role::User && !m.auto_injected)
                    .map(|m| m.content.clone());
                let assistant = session
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
                            let mut session = session_mutex.lock().await;
                            if session.title.is_empty() {
                                session.set_title(&title);
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
    fn prepare_history(&self, session: &mut Session, content: &str, sender: &str) -> String {
        let content = self.hook.preprocess(&session.agent, content);
        if sender.is_empty() {
            session.history.push(Message::user(&content));
        } else {
            session
                .history
                .push(Message::user_with_sender(&content, sender));
        }

        // Strip previous auto-injected messages to avoid accumulation.
        session.history.retain(|m| !m.auto_injected);

        let agent_name = session.agent.clone();
        let recall_msgs = self
            .hook
            .on_before_run(&agent_name, session.id, &session.history);
        if !recall_msgs.is_empty() {
            let insert_pos = session.history.len().saturating_sub(1);
            for (i, msg) in recall_msgs.into_iter().enumerate() {
                session.history.insert(insert_pos + i, msg);
            }
        }
        agent_name
    }

    /// Send a message to a session and run to completion.
    pub async fn send_to(
        &self,
        session_id: u64,
        content: &str,
        sender: &str,
    ) -> Result<AgentResponse> {
        let session_mutex = self
            .sessions
            .read()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("session {session_id} not found"))?;

        let mut session = session_mutex.lock().await;
        let pre_run_len = session.history.len();
        let agent_name = self.prepare_history(&mut session, content, sender);
        let agent_ref = self
            .agents
            .get(&session.agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", session.agent))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let run_start = std::time::Instant::now();
        self.active_sessions.write().await.insert(session_id);
        let response = agent_ref.run(&mut session.history, tx, None).await;
        self.active_sessions.write().await.remove(&session_id);
        session.uptime_secs += run_start.elapsed().as_secs();

        // Drain events, stash compact summary if one occurred.
        let mut compact_summary: Option<String> = None;
        while let Ok(event) = rx.try_recv() {
            if let AgentEvent::Compact { ref summary } = event {
                compact_summary = Some(summary.clone());
            }
            self.hook.on_event(&agent_name, session_id, &event);
        }

        // Append-only persistence.
        if let Some(summary) = compact_summary {
            // Compaction happened: append compact marker + post-compact messages.
            session.append_compact(&summary);
            // history[0] is the summary-as-user-message; skip it (compact line serves that role).
            if session.history.len() > 1 {
                session.append_messages(&session.history[1..]);
            }
        } else {
            // No compaction: append new messages since pre_run.
            session.append_messages(&session.history[pre_run_len..]);
        }

        // Persist updated uptime to meta line.
        session.rewrite_meta();

        // Generate title in background if this is the first exchange.
        if session.title.is_empty() && session.history.len() >= 2 {
            self.spawn_title_generation(session_id, session_mutex.clone());
        }
        Ok(response)
    }

    /// Send a message to a session and stream response events.
    pub fn stream_to(
        &self,
        session_id: u64,
        content: &str,
        sender: &str,
    ) -> impl Stream<Item = AgentEvent> + '_ {
        let content = content.to_owned();
        let sender = sender.to_owned();
        stream! {
            let session_mutex = match self
                .sessions
                .read()
                .await
                .get(&session_id)
                .cloned()
            {
                Some(m) => m,
                None => {
                    let resp = AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("session {session_id} not found"),
                        ),
                        steps: vec![],
                    };
                    yield AgentEvent::Done(resp);
                    return;
                }
            };

            let mut session = session_mutex.lock().await;
            let pre_run_len = session.history.len();
            let agent_name = self.prepare_history(&mut session, &content, &sender);
            let agent_ref = match self.agents.get(&session.agent) {
                Some(a) => a,
                None => {
                    let resp = AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("agent '{}' not registered", session.agent),
                        ),
                        steps: vec![],
                    };
                    yield AgentEvent::Done(resp);
                    return;
                }
            };

            let run_start = std::time::Instant::now();
            self.active_sessions.write().await.insert(session_id);
            let mut compact_summary: Option<String> = None;
            let mut done_event: Option<AgentEvent> = None;
            {
                let mut event_stream = std::pin::pin!(agent_ref.run_stream(&mut session.history, Some(session_id)));
                while let Some(event) = event_stream.next().await {
                    if let AgentEvent::Compact { ref summary } = event {
                        compact_summary = Some(summary.clone());
                    }
                    self.hook.on_event(&agent_name, session_id, &event);
                    // Hold back Done — yield it after persistence.
                    if matches!(event, AgentEvent::Done(_)) {
                        done_event = Some(event);
                    } else {
                        yield event;
                    }
                }
            }
            // Borrow on session.history is released. Persist now.
            self.active_sessions.write().await.remove(&session_id);
            session.uptime_secs += run_start.elapsed().as_secs();
            if let Some(summary) = compact_summary {
                session.append_compact(&summary);
                if session.history.len() > 1 {
                    session.append_messages(&session.history[1..]);
                }
            } else {
                session.append_messages(&session.history[pre_run_len..]);
            }
            // Persist updated uptime to meta line.
            session.rewrite_meta();

            // Generate title in background if this is the first exchange.
            if session.title.is_empty() && session.history.len() >= 2 {
                self.spawn_title_generation(session_id, session_mutex.clone());
            }
            // Now yield Done.
            if let Some(event) = done_event {
                yield event;
            }
        }
    }
}

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

    /// Create a new session for the given agent. Returns the session ID.
    pub async fn create_session(&self, agent: &str, created_by: &str) -> Result<u64> {
        if !self.agents.contains_key(agent) {
            bail!("agent '{agent}' not registered");
        }
        let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let session = Session::new(id, agent, created_by);
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
        let agent_name = self.prepare_history(&mut session, content, sender);
        let agent_ref = self
            .agents
            .get(&session.agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", session.agent))?;

        let (tx, mut rx) = mpsc::unbounded_channel();
        self.active_sessions.write().await.insert(session_id);
        let response = agent_ref.run(&mut session.history, tx, None).await;
        self.active_sessions.write().await.remove(&session_id);

        while let Ok(event) = rx.try_recv() {
            self.hook.on_event(&agent_name, session_id, &event);
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

            self.active_sessions.write().await.insert(session_id);
            let mut event_stream = std::pin::pin!(agent_ref.run_stream(&mut session.history, Some(session_id)));
            while let Some(event) = event_stream.next().await {
                self.hook.on_event(&agent_name, session_id, &event);
                yield event;
            }
            self.active_sessions.write().await.remove(&session_id);
        }
    }
}

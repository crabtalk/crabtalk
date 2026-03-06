//! Runtime — agent registry, schema store, and hook orchestration.
//!
//! [`Runtime`] holds agents in a `BTreeMap` with per-agent `Mutex` for
//! concurrent execution. Tool schemas are registered once at startup via
//! `hook.on_register_tools()` and stored in a plain [`ToolRegistry`].
//! Each agent receives a filtered schema snapshot and a [`ToolSender`] at
//! build time — the runtime holds no handlers or closures.

use crate::{
    Agent, AgentBuilder, AgentConfig, AgentEvent, AgentResponse, AgentStopReason,
    agent::tool::{ToolRegistry, ToolSender},
    model::{Message, Model},
    runtime::hook::Hook,
};
use anyhow::Result;
use async_stream::stream;
use compact_str::CompactString;
use futures_core::Stream;
use futures_util::StreamExt;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, mpsc};

pub mod hook;

/// The walrus runtime — agent registry, schema store, and hook orchestration.
///
/// Tool schemas are registered once at construction via `hook.on_register_tools()`.
/// Each agent is built with a filtered schema snapshot and a `ToolSender` so it
/// can dispatch tool calls back to the runtime without going through the runtime.
/// `Runtime::new()` is async — it calls `hook.on_register_tools()` during
/// construction to populate the schema registry.
pub struct Runtime<M: Model, H: Hook> {
    pub model: M,
    pub hook: H,
    agents: BTreeMap<CompactString, Arc<Mutex<Agent<M>>>>,
    tools: ToolRegistry,
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
            tools,
            tool_tx,
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
        let tools = self.tools.tools();
        let mut builder = AgentBuilder::new(self.model.clone())
            .config(config)
            .tools(tools);
        if let Some(tx) = &self.tool_tx {
            builder = builder.tool_tx(tx.clone());
        }
        let agent = builder.build();
        self.agents.insert(name, Arc::new(Mutex::new(agent)));
    }

    /// Get a registered agent's config by name (cloned).
    pub async fn agent(&self, name: &str) -> Option<AgentConfig> {
        let mutex = self.agents.get(name)?;
        Some(mutex.lock().await.config.clone())
    }

    /// Get all registered agent configs (cloned, alphabetical order).
    pub async fn agents(&self) -> Vec<AgentConfig> {
        let mut configs = Vec::with_capacity(self.agents.len());
        for mutex in self.agents.values() {
            configs.push(mutex.lock().await.config.clone());
        }
        configs
    }

    /// Get the per-agent mutex by name.
    pub fn agent_mutex(&self, name: &str) -> Option<Arc<Mutex<Agent<M>>>> {
        self.agents.get(name).cloned()
    }

    // --- Execution ---

    /// Send a message to an agent and run to completion.
    ///
    /// Locks the per-agent mutex, pushes the user message, delegates to
    /// `agent.run()`, and forwards all events to `hook.on_event()`.
    pub async fn send_to(&self, agent: &str, content: &str) -> Result<AgentResponse> {
        let mutex = self
            .agents
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not registered"))?;

        let mut guard = mutex.lock().await;
        guard.push_message(Message::user(content));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let response = guard.run(tx).await;

        while let Ok(event) = rx.try_recv() {
            self.hook.on_event(agent, &event);
        }

        Ok(response)
    }

    /// Send a message to an agent and stream response events.
    ///
    /// Locks the per-agent mutex, pushes the user message, and streams events
    /// forwarded to `hook.on_event()`.
    pub fn stream_to<'a>(
        &'a self,
        agent: &'a str,
        content: &'a str,
    ) -> impl Stream<Item = AgentEvent> + 'a {
        stream! {
            let mutex = match self.agents.get(agent) {
                Some(m) => m,
                None => {
                    let resp = AgentResponse {
                        final_response: None,
                        iterations: 0,
                        stop_reason: AgentStopReason::Error(
                            format!("agent '{agent}' not registered"),
                        ),
                        steps: vec![],
                    };
                    yield AgentEvent::Done(resp);
                    return;
                }
            };

            let mut guard = mutex.lock().await;
            guard.push_message(Message::user(content));

            let mut event_stream = std::pin::pin!(guard.run_stream());
            while let Some(event) = event_stream.next().await {
                self.hook.on_event(agent, &event);
                yield event;
            }
        }
    }
}

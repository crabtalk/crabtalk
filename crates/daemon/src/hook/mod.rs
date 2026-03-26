//! Thin hook wrapper for the daemon.
//!
//! [`DaemonHook`] wraps [`RuntimeHook<DaemonBridge>`](runtime::RuntimeHook)
//! and adds agent event broadcasting for console subscriptions.

pub mod bridge;

use bridge::DaemonBridge;
use runtime::RuntimeHook;
use std::sync::Arc;
use tokio::sync::broadcast;
use wcore::{
    AgentConfig, AgentEvent, Hook, ToolRegistry,
    model::Message,
    protocol::message::{AgentEventKind, AgentEventMsg},
};

pub struct DaemonHook {
    pub inner: RuntimeHook<DaemonBridge>,
    /// Broadcast channel for agent events (console subscription).
    events_tx: broadcast::Sender<AgentEventMsg>,
}

impl DaemonHook {
    /// Create a new DaemonHook wrapping a RuntimeHook.
    pub fn new(inner: RuntimeHook<DaemonBridge>) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self { inner, events_tx }
    }

    /// Subscribe to agent events (for console event streaming).
    pub fn subscribe_events(&self) -> broadcast::Receiver<AgentEventMsg> {
        self.events_tx.subscribe()
    }

    // --- Convenience accessors ---

    /// Access pending_asks on the bridge.
    pub fn pending_asks(
        &self,
    ) -> &Arc<
        tokio::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<String>>>,
    > {
        &self.inner.bridge.pending_asks
    }

    /// Access session_cwds on the bridge.
    pub fn session_cwds(
        &self,
    ) -> &Arc<tokio::sync::Mutex<std::collections::HashMap<u64, std::path::PathBuf>>> {
        &self.inner.bridge.session_cwds
    }

    /// Access memory.
    pub fn memory(&self) -> Option<&runtime::Memory> {
        self.inner.memory.as_ref()
    }

    /// Register an agent's scope.
    pub fn register_scope(&mut self, name: String, config: &AgentConfig) {
        self.inner.register_scope(name, config);
    }

    /// Route a tool call through the inner RuntimeHook.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        sender: &str,
        session_id: Option<u64>,
    ) -> String {
        self.inner
            .dispatch_tool(name, args, agent, sender, session_id)
            .await
    }
}

impl Hook for DaemonHook {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        self.inner.on_build_agent(config)
    }

    fn preprocess(&self, agent: &str, content: &str) -> String {
        self.inner.preprocess(agent, content)
    }

    fn on_before_run(&self, agent: &str, session_id: u64, history: &[Message]) -> Vec<Message> {
        self.inner.on_before_run(agent, session_id, history)
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.inner.on_register_tools(tools).await;
    }

    fn on_event(&self, agent: &str, session_id: u64, event: &AgentEvent) {
        let (kind, content) = match event {
            AgentEvent::TextDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent text delta");
                (AgentEventKind::TextDelta, String::new())
            }
            AgentEvent::ThinkingDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent thinking delta");
                (AgentEventKind::ThinkingDelta, String::new())
            }
            AgentEvent::ToolCallsBegin(_) => return,
            AgentEvent::ToolCallsStart(calls) => {
                tracing::debug!(%agent, count = calls.len(), "agent tool calls");
                let labels: Vec<String> = calls
                    .iter()
                    .map(|c| {
                        if c.function.name == "bash"
                            && let Ok(v) =
                                serde_json::from_str::<serde_json::Value>(&c.function.arguments)
                            && let Some(cmd) = v.get("command").and_then(|c| c.as_str())
                        {
                            return format!("bash({})", cmd.lines().next().unwrap_or(""));
                        }
                        c.function.name.clone()
                    })
                    .collect();
                (AgentEventKind::ToolStart, labels.join(", "))
            }
            AgentEvent::ToolResult { call_id, .. } => {
                tracing::debug!(%agent, %call_id, "agent tool result");
                (AgentEventKind::ToolResult, call_id.clone())
            }
            AgentEvent::ToolCallsComplete => {
                tracing::debug!(%agent, "agent tool calls complete");
                (AgentEventKind::ToolsComplete, String::new())
            }
            AgentEvent::Compact { summary } => {
                tracing::info!(%agent, summary_len = summary.len(), "context compacted");
                return;
            }
            AgentEvent::Done(response) => {
                tracing::info!(
                    %agent,
                    iterations = response.iterations,
                    stop_reason = ?response.stop_reason,
                    "agent run complete"
                );
                (AgentEventKind::Done, String::new())
            }
        };
        let _ = self.events_tx.send(AgentEventMsg {
            agent: agent.to_string(),
            session: session_id,
            kind: kind.into(),
            content,
        });
    }
}

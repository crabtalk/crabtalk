//! Node event types and dispatch.
//!
//! All inbound stimuli (socket, channel, tool calls) are represented as
//! [`NodeEvent`] variants sent through a single `mpsc::unbounded_channel`.
//! The [`Node`] processes them via [`handle_events`](Node::handle_events).
//!
//! Tool call routing is fully delegated to [`NodeEnv::dispatch_tool`] —
//! no tool name matching happens here.

use crate::node::Node;
use crabllm_core::Provider;
use futures_util::{StreamExt, pin_mut};
use runtime::host::Host;
use tokio::sync::{mpsc, oneshot};
use wcore::{
    AgentConfig, ToolRequest,
    protocol::{
        api::Server,
        message::{ClientMessage, ServerMessage},
    },
};

/// Inbound event from any source, processed by the central event loop.
pub enum NodeEvent {
    /// A client message from any source (socket, telegram).
    /// Reply channel streams `ServerMessage`s back to the caller.
    Message {
        /// The parsed client message.
        msg: ClientMessage,
        /// Per-request reply channel for streaming `ServerMessage`s back.
        reply: mpsc::Sender<ServerMessage>,
    },
    /// A tool call from an agent, routed through `NodeEnv::dispatch_tool`.
    ToolCall(ToolRequest),
    /// Publish an event to the event bus — fires matching subscriptions.
    PublishEvent {
        /// Namespaced source, e.g. `"agent:scout:done"`.
        source: String,
        /// JSON payload delivered as message content to target agents.
        payload: String,
    },
    /// Register an ephemeral agent for delegate dispatch.
    AddEphemeral {
        config: AgentConfig,
        reply: oneshot::Sender<()>,
    },
    /// Remove an ephemeral agent after delegate completion.
    RemoveEphemeral { name: String },
    /// Graceful shutdown request.
    Shutdown,
}

/// Shorthand for the event sender half of the daemon event channel.
pub type NodeEventSender = mpsc::UnboundedSender<NodeEvent>;

// ── Event dispatch ───────────────────────────────────────────────────

impl<P: Provider + 'static, H: Host + 'static> Node<P, H> {
    /// Process events until [`NodeEvent::Shutdown`] is received.
    ///
    /// Spawns a task for each event to avoid blocking on LLM calls.
    pub(crate) async fn handle_events(&self, mut rx: mpsc::UnboundedReceiver<NodeEvent>) {
        tracing::info!("event loop started");
        while let Some(event) = rx.recv().await {
            match event {
                NodeEvent::Message { msg, reply } => self.handle_message(msg, reply),
                NodeEvent::ToolCall(req) => self.handle_tool_call(req),
                NodeEvent::PublishEvent { source, payload } => {
                    self.events.lock().await.publish(&source, &payload);
                }
                NodeEvent::AddEphemeral { config, reply } => {
                    let rt = self.runtime.read().await.clone();
                    rt.add_ephemeral(config).await;
                    let _ = reply.send(());
                }
                NodeEvent::RemoveEphemeral { name } => {
                    let rt = self.runtime.read().await.clone();
                    rt.remove_ephemeral(&name).await;
                }
                NodeEvent::Shutdown => {
                    tracing::info!("event loop shutting down");
                    break;
                }
            }
        }
        tracing::info!("event loop stopped");
    }

    /// Dispatch a client message through the Server trait and stream replies.
    fn handle_message(&self, msg: ClientMessage, reply: mpsc::Sender<ServerMessage>) {
        let daemon = self.clone();
        tokio::spawn(async move {
            let stream = daemon.dispatch(msg);
            pin_mut!(stream);
            while let Some(server_msg) = stream.next().await {
                if reply.send(server_msg).await.is_err() {
                    break;
                }
            }
        });
    }

    /// Route a tool call through `Env::dispatch_tool`.
    fn handle_tool_call(&self, req: ToolRequest) {
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            tracing::debug!(tool = %req.name, agent = %req.agent, "tool dispatch");
            let rt = runtime.read().await.clone();
            let result = rt
                .hook
                .dispatch_tool(
                    &req.name,
                    &req.args,
                    &req.agent,
                    &req.sender,
                    req.conversation_id,
                )
                .await;
            let _ = req.reply.send(result);
        });
    }
}

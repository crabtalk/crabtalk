//! Daemon event types and dispatch.
//!
//! All inbound stimuli (socket, channel, tool calls) are represented as
//! [`DaemonEvent`] variants sent through a single `mpsc::unbounded_channel`.
//! The [`Daemon`] processes them via [`handle_events`](Daemon::handle_events).
//!
//! Tool call routing is fully delegated to [`DaemonEnv::dispatch_tool`] —
//! no tool name matching happens here.

use crate::daemon::Daemon;
use futures_util::{StreamExt, pin_mut};
use runtime::backend::Backend;
use tokio::sync::mpsc;
use wcore::{
    ToolRequest,
    protocol::{
        api::Server,
        message::{ClientMessage, ServerMessage},
    },
};

/// Inbound event from any source, processed by the central event loop.
pub enum DaemonEvent {
    /// A client message from any source (socket, telegram).
    /// Reply channel streams `ServerMessage`s back to the caller.
    Message {
        /// The parsed client message.
        msg: ClientMessage,
        /// Per-request reply channel for streaming `ServerMessage`s back.
        reply: mpsc::Sender<ServerMessage>,
    },
    /// A tool call from an agent, routed through `DaemonEnv::dispatch_tool`.
    ToolCall(ToolRequest),
    /// Graceful shutdown request.
    Shutdown,
}

/// Shorthand for the event sender half of the daemon event channel.
pub type DaemonEventSender = mpsc::UnboundedSender<DaemonEvent>;

// ── Event dispatch ───────────────────────────────────────────────────

impl<B: Backend + 'static> Daemon<B> {
    /// Process events until [`DaemonEvent::Shutdown`] is received.
    ///
    /// Spawns a task for each event to avoid blocking on LLM calls.
    pub(crate) async fn handle_events(&self, mut rx: mpsc::UnboundedReceiver<DaemonEvent>) {
        tracing::info!("event loop started");
        while let Some(event) = rx.recv().await {
            match event {
                DaemonEvent::Message { msg, reply } => self.handle_message(msg, reply),
                DaemonEvent::ToolCall(req) => self.handle_tool_call(req),
                DaemonEvent::Shutdown => {
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
                    req.session_id,
                )
                .await;
            let _ = req.reply.send(result);
        });
    }
}

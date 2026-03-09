//! Daemon event types and dispatch.
//!
//! All inbound stimuli (socket, channel, tool calls) are represented as
//! [`DaemonEvent`] variants sent through a single `mpsc::unbounded_channel`.
//! The [`Daemon`] processes them via [`handle_events`](Daemon::handle_events).
//!
//! Tool call routing is fully delegated to [`DaemonHook::dispatch_tool`] —
//! no tool name matching happens here.

use crate::daemon::Daemon;
use compact_str::CompactString;
use futures_util::{StreamExt, pin_mut};
use tokio::sync::mpsc;
use wcore::{
    ToolRequest,
    protocol::{
        api::Server,
        message::{client::ClientMessage, server::ServerMessage},
    },
};

/// Inbound event from any source, processed by the central event loop.
pub enum DaemonEvent {
    /// A client message from any source (socket, telegram, discord).
    /// Reply channel streams `ServerMessage`s back to the caller.
    Message {
        /// The parsed client message.
        msg: ClientMessage,
        /// Per-request reply channel for streaming `ServerMessage`s back.
        reply: mpsc::UnboundedSender<ServerMessage>,
    },
    /// A tool call from an agent, routed through `DaemonHook::dispatch_tool`.
    ToolCall(ToolRequest),
    /// Periodic heartbeat tick.
    Heartbeat,
    /// Graceful shutdown request.
    Shutdown,
}

/// Shorthand for the event sender half of the daemon event channel.
pub type DaemonEventSender = mpsc::UnboundedSender<DaemonEvent>;

// ── Event dispatch ───────────────────────────────────────────────────

impl Daemon {
    /// Process events until [`DaemonEvent::Shutdown`] is received.
    ///
    /// Spawns a task for each event to avoid blocking on LLM calls.
    pub(crate) async fn handle_events(&self, mut rx: mpsc::UnboundedReceiver<DaemonEvent>) {
        tracing::info!("event loop started");
        while let Some(event) = rx.recv().await {
            match event {
                DaemonEvent::Message { msg, reply } => self.handle_message(msg, reply),
                DaemonEvent::ToolCall(req) => self.handle_tool_call(req),
                DaemonEvent::Heartbeat => self.handle_heartbeat(),
                DaemonEvent::Shutdown => {
                    tracing::info!("event loop shutting down");
                    break;
                }
            }
        }
        tracing::info!("event loop stopped");
    }

    /// Dispatch a client message through the Server trait and stream replies.
    fn handle_message(&self, msg: ClientMessage, reply: mpsc::UnboundedSender<ServerMessage>) {
        let daemon = self.clone();
        tokio::spawn(async move {
            let stream = daemon.dispatch(msg);
            pin_mut!(stream);
            while let Some(server_msg) = stream.next().await {
                if reply.send(server_msg).is_err() {
                    break;
                }
            }
        });
    }

    /// Handle a heartbeat tick: deliver queued create_task entries and promote spawn_task entries.
    fn handle_heartbeat(&self) {
        let daemon = self.clone();
        tokio::spawn(async move {
            tracing::debug!("heartbeat tick");
            let rt = daemon.runtime.read().await.clone();
            let tasks_arc = rt.hook.tasks.clone();

            // Step 1: Gather queued create_task entries grouped by agent.
            let groups = {
                let registry = tasks_arc.lock().await;
                registry.queued_create_tasks()
            };

            // For each agent group: format task list, send as message.
            for (agent, task_entries) in &groups {
                if task_entries.is_empty() {
                    continue;
                }
                let task_context: String = task_entries
                    .iter()
                    .map(|(id, desc)| format!("- Task #{id}: {desc}"))
                    .collect::<Vec<_>>()
                    .join("\n");

                let content = if daemon.heartbeat_prompt.is_empty() {
                    format!("You have pending tasks:\n{task_context}")
                } else {
                    format!(
                        "{}\n\nPending tasks:\n{task_context}",
                        daemon.heartbeat_prompt
                    )
                };

                // Mark tasks InProgress.
                {
                    let mut registry = tasks_arc.lock().await;
                    for (id, _) in task_entries {
                        registry.set_status(*id, crate::hook::task::TaskStatus::InProgress);
                    }
                }

                // Dispatch via event channel with created_by: "heartbeat".
                let msg = ClientMessage::Send {
                    agent: CompactString::from(agent.as_str()),
                    content,
                    session: None,
                };
                let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
                let _ = daemon.event_tx.send(DaemonEvent::Message {
                    msg,
                    reply: reply_tx,
                });
            }

            // Step 2: Promote queued spawn_task entries.
            {
                let reg = tasks_arc.clone();
                tasks_arc.lock().await.promote_next(reg);
            }
        });
    }

    /// Route a tool call through `DaemonHook::dispatch_tool`.
    fn handle_tool_call(&self, req: ToolRequest) {
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            tracing::debug!(tool = %req.name, agent = %req.agent, "tool dispatch");
            let rt = runtime.read().await.clone();
            let result = rt
                .hook
                .dispatch_tool(&req.name, &req.args, &req.agent, req.task_id)
                .await;
            let _ = req.reply.send(result);
        });
    }
}

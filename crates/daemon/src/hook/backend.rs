//! DaemonBackend — server-specific Backend implementation.
//!
//! Provides `ask_user` and `delegate` dispatch using daemon event channels,
//! per-session CWD resolution, and agent event broadcasting.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use runtime::backend::Backend;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use wcore::{
    AgentEvent,
    protocol::message::{AgentEventKind, AgentEventMsg, ClientMessage, SendMsg, server_message},
};

/// Timeout for waiting on user reply (5 minutes).
const ASK_USER_TIMEOUT: Duration = Duration::from_secs(300);

/// Server-specific backend for the daemon. Owns event channels and session state.
pub struct DaemonBackend {
    /// Event channel for task delegation.
    pub event_tx: DaemonEventSender,
    /// Pending `ask_user` oneshots, keyed by session_id.
    pub pending_asks: Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>,
    /// Per-session working directory overrides.
    pub session_cwds: Arc<Mutex<HashMap<u64, PathBuf>>>,
    /// Broadcast channel for agent events (console subscription).
    pub events_tx: broadcast::Sender<AgentEventMsg>,
}

impl DaemonBackend {
    /// Subscribe to agent events (for console event streaming).
    pub fn subscribe_events(&self) -> broadcast::Receiver<AgentEventMsg> {
        self.events_tx.subscribe()
    }
}

impl Backend for DaemonBackend {
    async fn dispatch_ask_user(&self, args: &str, session_id: Option<u64>) -> String {
        let input: runtime::ask_user::AskUser = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };

        let session_id = match session_id {
            Some(id) => id,
            None => return "ask_user is only available in streaming mode".to_owned(),
        };

        let (tx, rx) = oneshot::channel();
        self.pending_asks.lock().await.insert(session_id, tx);

        match tokio::time::timeout(ASK_USER_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => {
                self.pending_asks.lock().await.remove(&session_id);
                "ask_user cancelled: reply channel closed".to_owned()
            }
            Err(_) => {
                self.pending_asks.lock().await.remove(&session_id);
                let headers: Vec<&str> =
                    input.questions.iter().map(|q| q.header.as_str()).collect();
                format!(
                    "ask_user timed out after {}s: no reply received for: {}",
                    ASK_USER_TIMEOUT.as_secs(),
                    headers.join("; "),
                )
            }
        }
    }

    async fn dispatch_delegate(&self, args: &str, _agent: &str) -> String {
        let input: runtime::task::Delegate = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };

        let mut handles = Vec::with_capacity(input.tasks.len());
        for task in input.tasks {
            let handle = spawn_agent_task(task.agent.clone(), task.message, self.event_tx.clone());
            handles.push((task.agent, handle));
        }

        let mut results = Vec::with_capacity(handles.len());
        for (agent_name, handle) in handles {
            let (result, error) = match handle.await {
                Ok((r, e)) => (r, e),
                Err(e) => (None, Some(format!("task panicked: {e}"))),
            };
            results.push(serde_json::json!({
                "agent": agent_name,
                "result": result,
                "error": error,
            }));
        }

        serde_json::to_string(&results).unwrap_or_else(|e| format!("serialization error: {e}"))
    }

    fn session_cwd(&self, session_id: u64) -> Option<PathBuf> {
        self.session_cwds
            .try_lock()
            .ok()
            .and_then(|m| m.get(&session_id).cloned())
    }

    fn on_agent_event(&self, agent: &str, session_id: u64, event: &AgentEvent) {
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
            AgentEvent::ToolResult {
                call_id,
                duration_ms,
                ..
            } => {
                tracing::debug!(%agent, %call_id, %duration_ms, "agent tool result");
                (AgentEventKind::ToolResult, format!("{duration_ms}ms"))
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
                    stop_reason = %response.stop_reason,
                    "agent run complete"
                );
                let content = format_usage(response);
                (AgentEventKind::Done, content)
            }
        };
        let _ = self.events_tx.send(AgentEventMsg {
            agent: agent.to_string(),
            session: session_id,
            kind: kind.into(),
            content,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }
}

/// Spawn an agent task via the event channel and collect its response.
fn spawn_agent_task(
    agent: String,
    message: String,
    event_tx: DaemonEventSender,
) -> tokio::task::JoinHandle<(Option<String>, Option<String>)> {
    tokio::spawn(async move {
        let (reply_tx, mut reply_rx) = mpsc::channel(transport::REPLY_CHANNEL_CAPACITY);
        let msg = ClientMessage::from(SendMsg {
            agent,
            content: message,
            session: None,
            sender: None,
            cwd: None,
            new_chat: false,
            resume_file: None,
        });
        if event_tx
            .send(DaemonEvent::Message {
                msg,
                reply: reply_tx,
            })
            .is_err()
        {
            return (None, Some("event channel closed".to_owned()));
        }

        let mut result_content: Option<String> = None;
        let mut error_msg: Option<String> = None;
        let mut session_id: Option<u64> = None;

        while let Some(msg) = reply_rx.recv().await {
            match msg.msg {
                Some(server_message::Msg::Response(resp)) => {
                    session_id = Some(resp.session);
                    result_content = Some(resp.content);
                }
                Some(server_message::Msg::Error(err)) => {
                    error_msg = Some(err.message);
                }
                _ => {}
            }
        }

        if let Some(sid) = session_id {
            // Capacity 1: receiver is dropped immediately, so the first
            // send() in handle_message returns Err and the loop exits.
            let (reply_tx, _) = mpsc::channel(1);
            let _ = event_tx.send(DaemonEvent::Message {
                msg: ClientMessage {
                    msg: Some(wcore::protocol::message::client_message::Msg::Kill(
                        wcore::protocol::message::KillMsg { session: sid },
                    )),
                },
                reply: reply_tx,
            });
        }

        (result_content, error_msg)
    })
}

fn format_usage(response: &wcore::AgentResponse) -> String {
    if response.steps.is_empty() {
        return String::new();
    }
    let mut prompt = 0u32;
    let mut completion = 0u32;
    let mut cache_hit = 0u32;
    for step in &response.steps {
        let u = &step.response.usage;
        prompt += u.prompt_tokens;
        completion += u.completion_tokens;
        if let Some(v) = u.prompt_cache_hit_tokens {
            cache_hit += v;
        }
    }
    let model = &response.model;
    if cache_hit > 0 {
        format!(
            "{model} {} in ({} cached) / {} out",
            human_tokens(prompt),
            human_tokens(cache_hit),
            human_tokens(completion),
        )
    } else {
        format!(
            "{model} {} in / {} out",
            human_tokens(prompt),
            human_tokens(completion),
        )
    }
}

fn human_tokens(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

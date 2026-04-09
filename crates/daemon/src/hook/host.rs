//! DaemonHost — server-specific Host implementation.
//!
//! Provides `ask_user` and `delegate` dispatch using daemon event channels,
//! per-session CWD resolution, and agent event broadcasting.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use runtime::host::Host;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use wcore::{
    AgentEvent,
    protocol::message::{
        AgentEventKind, AgentEventMsg, ClientMessage, SendMsg, ToolCallInfo, server_message,
    },
};

/// Tool result output is truncated to this many bytes in the broadcast.
/// Keeps the firehose lightweight while still giving rich UIs enough
/// content to render meaningful previews.
const MAX_TOOL_OUTPUT_BROADCAST: usize = 2048;

/// Timeout for waiting on user reply (5 minutes).
const ASK_USER_TIMEOUT: Duration = Duration::from_secs(300);

/// Server-specific host for the daemon. Owns event channels and session state.
#[derive(Clone)]
pub struct DaemonHost {
    /// Event channel for task delegation.
    pub(crate) event_tx: DaemonEventSender,
    /// Pending `ask_user` oneshots, keyed by conversation_id.
    pub(crate) pending_asks: Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>,
    /// Per-conversation working directory overrides.
    pub(crate) conversation_cwds: Arc<Mutex<HashMap<u64, PathBuf>>>,
    /// Broadcast channel for agent events (console subscription).
    pub(crate) events_tx: broadcast::Sender<AgentEventMsg>,
}

impl Host for DaemonHost {
    async fn dispatch_ask_user(
        &self,
        args: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        let input: runtime::ask_user::AskUser =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

        let conversation_id =
            conversation_id.ok_or("ask_user is only available in streaming mode")?;

        let (tx, rx) = oneshot::channel();
        self.pending_asks.lock().await.insert(conversation_id, tx);

        match tokio::time::timeout(ASK_USER_TIMEOUT, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => {
                self.pending_asks.lock().await.remove(&conversation_id);
                Err("ask_user cancelled: reply channel closed".to_owned())
            }
            Err(_) => {
                self.pending_asks.lock().await.remove(&conversation_id);
                let headers: Vec<&str> =
                    input.questions.iter().map(|q| q.header.as_str()).collect();
                Err(format!(
                    "ask_user timed out after {}s: no reply received for: {}",
                    ASK_USER_TIMEOUT.as_secs(),
                    headers.join("; "),
                ))
            }
        }
    }

    async fn dispatch_delegate(&self, args: &str, _agent: &str) -> Result<String, String> {
        let input: runtime::task::Delegate =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

        // Register ephemeral agents and resolve agent names.
        let mut ephemeral_names = Vec::new();
        let mut tasks = Vec::with_capacity(input.tasks.len());
        for task in input.tasks {
            let agent_name = if let Some(prompt) = task.system_prompt {
                let name = if task.agent.is_empty() {
                    ephemeral_agent_name()
                } else {
                    task.agent
                };
                let mut config = wcore::AgentConfig::new(&name);
                config.system_prompt = prompt;
                let (tx, rx) = oneshot::channel();
                let _ = self
                    .event_tx
                    .send(DaemonEvent::AddEphemeral { config, reply: tx });
                let _ = rx.await;
                ephemeral_names.push(name.clone());
                name
            } else {
                task.agent
            };

            let sender = delegate_sender();
            let handle = spawn_agent_task(
                agent_name.clone(),
                task.message,
                task.cwd,
                sender.clone(),
                self.event_tx.clone(),
            );
            tasks.push((agent_name, sender, handle));
        }

        if input.background {
            let mut json_results = Vec::with_capacity(tasks.len());
            let mut handles = Vec::with_capacity(tasks.len());
            for (agent, sender, handle) in tasks {
                json_results.push(serde_json::json!({ "agent": agent, "task_id": sender }));
                handles.push(handle);
            }
            // Spawn cleanup that waits for all delegates to finish.
            if !ephemeral_names.is_empty() {
                let event_tx = self.event_tx.clone();
                tokio::spawn(async move {
                    for h in handles {
                        let _ = h.await;
                    }
                    for name in ephemeral_names {
                        let _ = event_tx.send(DaemonEvent::RemoveEphemeral { name });
                    }
                });
            }
            return serde_json::to_string(&json_results)
                .map_err(|e| format!("serialization error: {e}"));
        }

        let mut results = Vec::with_capacity(tasks.len());
        for (agent_name, _sender, handle) in tasks {
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

        // Clean up ephemeral agents after foreground tasks complete.
        for name in ephemeral_names {
            let _ = self.event_tx.send(DaemonEvent::RemoveEphemeral { name });
        }

        serde_json::to_string(&results).map_err(|e| format!("serialization error: {e}"))
    }

    fn conversation_cwd(&self, conversation_id: u64) -> Option<PathBuf> {
        self.conversation_cwds
            .try_lock()
            .ok()
            .and_then(|m| m.get(&conversation_id).cloned())
    }

    fn on_agent_event(&self, agent: &str, conversation_id: u64, event: &AgentEvent) {
        /// Kind-specific payload built per match arm. `kind` is required —
        /// no `Default` impl, so the compiler forces every arm to set it.
        /// The other fields default to empty via struct update syntax.
        struct Payload {
            kind: AgentEventKind,
            content: String,
            tool_calls: Vec<ToolCallInfo>,
            tool_output: String,
            tool_is_error: bool,
        }

        impl Payload {
            fn of(kind: AgentEventKind) -> Self {
                Self {
                    kind,
                    content: String::new(),
                    tool_calls: Vec::new(),
                    tool_output: String::new(),
                    tool_is_error: false,
                }
            }
        }

        let p = match event {
            AgentEvent::TextStart => Payload::of(AgentEventKind::TextStart),
            AgentEvent::TextDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent text delta");
                Payload {
                    content: text.clone(),
                    ..Payload::of(AgentEventKind::TextDelta)
                }
            }
            AgentEvent::TextEnd => Payload::of(AgentEventKind::TextEnd),
            AgentEvent::ThinkingStart => Payload::of(AgentEventKind::ThinkingStart),
            AgentEvent::ThinkingDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent thinking delta");
                Payload {
                    content: text.clone(),
                    ..Payload::of(AgentEventKind::ThinkingDelta)
                }
            }
            AgentEvent::ThinkingEnd => Payload::of(AgentEventKind::ThinkingEnd),
            AgentEvent::ToolCallsBegin(_) => return,
            AgentEvent::ToolCallsStart(calls) => {
                tracing::debug!(%agent, count = calls.len(), "agent tool calls");
                // Single pass over `calls` builds both the human label and
                // the structured copy.
                let mut labels = Vec::with_capacity(calls.len());
                let mut structured = Vec::with_capacity(calls.len());
                for c in calls {
                    labels.push(tool_call_label(c));
                    structured.push(ToolCallInfo {
                        name: c.function.name.to_string(),
                        arguments: c.function.arguments.clone(),
                    });
                }
                Payload {
                    content: labels.join(", "),
                    tool_calls: structured,
                    ..Payload::of(AgentEventKind::ToolStart)
                }
            }
            AgentEvent::ToolResult {
                call_id,
                output,
                duration_ms,
            } => {
                let is_error = output.is_err();
                let text: &str = match output {
                    Ok(s) | Err(s) => s,
                };
                tracing::debug!(%agent, %call_id, %duration_ms, is_error, "agent tool result");
                Payload {
                    content: format!("{duration_ms}ms"),
                    tool_output: truncate_for_broadcast(text, MAX_TOOL_OUTPUT_BROADCAST),
                    tool_is_error: is_error,
                    ..Payload::of(AgentEventKind::ToolResult)
                }
            }
            AgentEvent::ToolCallsComplete => {
                tracing::debug!(%agent, "agent tool calls complete");
                Payload::of(AgentEventKind::ToolsComplete)
            }
            AgentEvent::Compact { summary } => {
                tracing::info!(%agent, summary_len = summary.len(), "context compacted");
                return;
            }
            AgentEvent::UserSteered { content } => {
                tracing::info!(%agent, content_len = content.len(), "user steered session");
                return;
            }
            AgentEvent::Done(response) => {
                tracing::info!(
                    %agent,
                    iterations = response.iterations,
                    stop_reason = %response.stop_reason,
                    "agent run complete"
                );
                Payload {
                    content: format_usage(response),
                    ..Payload::of(AgentEventKind::Done)
                }
            }
        };
        // The sender field is derived from the conversation's created_by.
        // Since we don't have access to conversation state here, we use
        // conversation_id as a string placeholder — subscribers correlate
        // by agent name.
        let _ = self.events_tx.send(AgentEventMsg {
            agent: agent.to_string(),
            sender: conversation_id.to_string(),
            kind: p.kind.into(),
            content: p.content,
            timestamp: chrono::Utc::now().to_rfc3339(),
            tool_calls: p.tool_calls,
            tool_output: p.tool_output,
            tool_is_error: p.tool_is_error,
        });

        // Publish agent completion to the event bus.
        if let AgentEvent::Done(response) = event {
            let payload = response.final_response.clone().unwrap_or_default();
            let _ = self.event_tx.send(DaemonEvent::PublishEvent {
                source: format!("agent:{}:done", agent),
                payload,
            });
        }
    }

    async fn reply_to_ask(&self, session: u64, content: String) -> anyhow::Result<bool> {
        if let Some(tx) = self.pending_asks.lock().await.remove(&session) {
            let _ = tx.send(content);
            return Ok(true);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(tx) = self.pending_asks.lock().await.remove(&session) {
            let _ = tx.send(content);
            return Ok(true);
        }
        Ok(false)
    }

    async fn set_conversation_cwd(&self, conversation: u64, cwd: std::path::PathBuf) {
        self.conversation_cwds
            .lock()
            .await
            .insert(conversation, cwd);
    }

    async fn clear_conversation_state(&self, conversation: u64) {
        self.pending_asks.lock().await.remove(&conversation);
        self.conversation_cwds.lock().await.remove(&conversation);
    }

    fn subscribe_events(&self) -> Option<broadcast::Receiver<AgentEventMsg>> {
        Some(self.events_tx.subscribe())
    }

    fn discover_instructions(&self, cwd: &Path) -> Option<String> {
        discover_instructions(cwd)
    }
}

/// Collect layered `Crab.md` instructions: global
/// (`~/.crabtalk/Crab.md`) first, then any `Crab.md` files found
/// walking up from `cwd` (root-first, project-last so project
/// instructions take precedence). Paths under the config dir are
/// skipped on the walk so a user who runs crabtalk from
/// `~/.crabtalk/` doesn't double-count the global file.
fn discover_instructions(cwd: &Path) -> Option<String> {
    let config_dir = &*wcore::paths::CONFIG_DIR;
    let mut layers = Vec::new();

    let global = config_dir.join("Crab.md");
    if let Ok(content) = std::fs::read_to_string(&global) {
        layers.push(content);
    }

    let mut found = Vec::new();
    let mut dir = cwd;
    loop {
        let candidate = dir.join("Crab.md");
        if candidate.is_file()
            && !candidate.starts_with(config_dir)
            && let Ok(content) = std::fs::read_to_string(&candidate)
        {
            found.push(content);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    found.reverse();
    layers.extend(found);

    if layers.is_empty() {
        return None;
    }
    Some(layers.join("\n\n"))
}

/// Generate a unique delegate sender identity.
fn delegate_sender() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("delegate:{id}")
}

/// Generate a unique ephemeral agent name.
fn ephemeral_agent_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("_ephemeral:{id}")
}

/// Spawn an agent task via the event channel and collect its response.
fn spawn_agent_task(
    agent: String,
    message: String,
    cwd: Option<String>,
    delegate_sender: String,
    event_tx: DaemonEventSender,
) -> tokio::task::JoinHandle<(Option<String>, Option<String>)> {
    tokio::spawn(async move {
        let (reply_tx, mut reply_rx) = mpsc::channel(transport::REPLY_CHANNEL_CAPACITY);
        let msg = ClientMessage::from(SendMsg {
            agent: agent.clone(),
            content: message,
            sender: Some(delegate_sender.clone()),
            cwd,
            guest: None,
            tool_choice: None,
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

        while let Some(msg) = reply_rx.recv().await {
            match msg.msg {
                Some(server_message::Msg::Response(resp)) => {
                    result_content = Some(resp.content);
                }
                Some(server_message::Msg::Error(err)) => {
                    error_msg = Some(err.message);
                }
                _ => {}
            }
        }

        // Kill the delegate conversation after completion.
        let (reply_tx, _) = mpsc::channel(1);
        let _ = event_tx.send(DaemonEvent::Message {
            msg: ClientMessage {
                msg: Some(wcore::protocol::message::client_message::Msg::Kill(
                    wcore::protocol::message::KillMsg {
                        agent,
                        sender: delegate_sender,
                    },
                )),
            },
            reply: reply_tx,
        });

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
        let u = &step.usage;
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

/// Build the human-readable label for a single tool call. Bash gets a
/// special preview of its first line; everything else falls back to the
/// function name. Used by the legacy `content` field for display-only
/// consumers — rich UIs should read `tool_calls` directly.
fn tool_call_label(c: &wcore::model::ToolCall) -> String {
    if c.function.name == "bash"
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&c.function.arguments)
        && let Some(cmd) = v.get("command").and_then(|c| c.as_str())
    {
        return format!("bash({})", cmd.lines().next().unwrap_or(""));
    }
    c.function.name.clone()
}

/// Truncate a tool output to at most `max` bytes for the event broadcast,
/// snapping back to a UTF-8 char boundary and appending an elision marker
/// if anything was dropped. Keeps the firehose lightweight.
///
/// If `max` is smaller than the marker itself, returns just the marker
/// (which may exceed `max`). Caller is expected to size `max` generously
/// — the helper exists to cap pathological multi-MB tool outputs, not
/// to enforce a precise byte budget.
fn truncate_for_broadcast(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let marker = "…[truncated]";
    if max <= marker.len() {
        return marker.to_owned();
    }
    let mut end = max - marker.len();
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{marker}", &s[..end])
}

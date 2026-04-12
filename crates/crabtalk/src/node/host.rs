//! NodeEnv — server-specific Host implementation and NodeEnv type alias.

use crate::node::hook::{ConversationCwds, NodeHook};
use runtime::Env;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::broadcast;
use wcore::{
    AgentEvent,
    protocol::message::{AgentEventKind, AgentEventMsg, ToolCallInfo},
};

/// Tool result output is truncated to this many bytes in the broadcast.
const MAX_TOOL_OUTPUT_BROADCAST: usize = 2048;

/// Server-specific host for the daemon — event broadcasting and
/// instruction discovery.
#[derive(Clone)]
pub struct NodeEnv {
    /// Broadcast channel for agent events (console subscription).
    pub(crate) events_tx: broadcast::Sender<AgentEventMsg>,
    /// Base working directory.
    pub(crate) cwd: PathBuf,
    /// Per-conversation CWD overrides (shared with NodeHook + OsHook + DelegateHook).
    pub(crate) conversation_cwds: ConversationCwds,
    /// Composite hook owning all sub-hooks and shared state.
    pub(crate) hook: Arc<NodeHook>,
}

impl Env for NodeEnv {
    type Hook = NodeHook;

    fn hook(&self) -> &NodeHook {
        &self.hook
    }

    fn on_agent_event(&self, agent: &str, conversation_id: u64, event: &AgentEvent) {
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
    }

    fn subscribe_events(&self) -> Option<broadcast::Receiver<AgentEventMsg>> {
        Some(self.events_tx.subscribe())
    }

    fn discover_instructions(&self, cwd: &Path) -> Option<String> {
        discover_instructions(cwd)
    }

    fn effective_cwd(&self, conversation_id: u64) -> PathBuf {
        if let Ok(map) = self.conversation_cwds.try_lock()
            && let Some(cwd) = map.get(&conversation_id)
        {
            return cwd.clone();
        }
        self.cwd.clone()
    }
}

impl wcore::ToolDispatcher for NodeEnv {
    fn dispatch<'a>(
        &'a self,
        name: &'a str,
        args: &'a str,
        agent: &'a str,
        sender: &'a str,
        conversation_id: Option<u64>,
    ) -> wcore::ToolFuture<'a> {
        runtime::env::dispatch_tool(self, name, args, agent, sender, conversation_id)
    }
}

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

fn tool_call_label(c: &wcore::model::ToolCall) -> String {
    if c.function.name == "bash"
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&c.function.arguments)
        && let Some(cmd) = v.get("command").and_then(|c| c.as_str())
    {
        return format!("bash({})", cmd.lines().next().unwrap_or(""));
    }
    c.function.name.clone()
}

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

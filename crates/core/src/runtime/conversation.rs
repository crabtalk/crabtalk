//! Conversation — pure working-context container.
//!
//! `Conversation` holds the in-memory state of an agent conversation:
//! metadata, history, and a session handle for persistence. All
//! persistence is delegated to the [`SessionRepo`] — the conversation
//! itself does not know about storage keys, step counters, or file
//! layouts.

use crate::{AgentEvent, AgentStep, model::HistoryEntry, repos::SessionHandle};
use crabllm_core::Usage;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Conversation metadata persisted alongside the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub agent: String,
    pub created_by: String,
    pub created_at: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub uptime_secs: u64,
}

/// A trace entry persisted alongside messages.
///
/// Captures the *how* of agent execution (which tools ran, how long
/// they took, why the agent stopped, what it cost) — information that
/// doesn't fit in the message stream itself but is invaluable for
/// debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventLine {
    /// One round of tool calls dispatched by the model.
    ToolStart {
        calls: Vec<ToolCallTrace>,
        ts: String,
    },
    /// A single tool call completed.
    ToolResult {
        call_id: String,
        duration_ms: u64,
        ts: String,
    },
    /// Agent run finished — final metadata and token usage.
    Done {
        model: String,
        iterations: usize,
        stop_reason: String,
        usage: Usage,
        ts: String,
    },
    /// User steered the agent mid-stream.
    UserSteered { content: String, ts: String },
}

/// Compact tool call info for [`EventLine::ToolStart`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTrace {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arguments: String,
}

impl EventLine {
    /// Build a trace entry from an [`AgentEvent`]. Returns `None` for
    /// events that don't carry useful trace information.
    pub fn from_agent_event(event: &AgentEvent) -> Option<Self> {
        let ts = chrono::Utc::now().to_rfc3339();
        match event {
            AgentEvent::ToolCallsStart(calls) => Some(Self::ToolStart {
                calls: calls
                    .iter()
                    .map(|c| ToolCallTrace {
                        id: c.id.clone(),
                        name: c.function.name.to_string(),
                        arguments: c.function.arguments.clone(),
                    })
                    .collect(),
                ts,
            }),
            AgentEvent::ToolResult {
                call_id,
                duration_ms,
                ..
            } => Some(Self::ToolResult {
                call_id: call_id.clone(),
                duration_ms: *duration_ms,
                ts,
            }),
            AgentEvent::Done(resp) => Some(Self::Done {
                model: resp.model.clone(),
                iterations: resp.iterations,
                stop_reason: resp.stop_reason.to_string(),
                usage: sum_step_usage(&resp.steps),
                ts,
            }),
            AgentEvent::UserSteered { content } => Some(Self::UserSteered {
                content: content.clone(),
                ts,
            }),
            _ => None,
        }
    }
}

/// Sum token usage across all steps.
fn sum_step_usage(steps: &[AgentStep]) -> Usage {
    steps.iter().fold(Usage::default(), |mut acc, step| {
        let u = &step.usage;
        acc.prompt_tokens += u.prompt_tokens;
        acc.completion_tokens += u.completion_tokens;
        acc.total_tokens += u.total_tokens;
        if let Some(v) = u.prompt_cache_hit_tokens {
            *acc.prompt_cache_hit_tokens.get_or_insert(0) += v;
        }
        if let Some(v) = u.prompt_cache_miss_tokens {
            *acc.prompt_cache_miss_tokens.get_or_insert(0) += v;
        }
        acc
    })
}

/// A compaction archive segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveSegment {
    pub title: String,
    pub summary: String,
    pub archived_at: String,
}

/// A conversation tied to a specific agent.
///
/// Pure working-context container. Persistence is delegated to the
/// [`SessionRepo`](crate::repos::SessionRepo) via the session handle.
#[derive(Debug, Clone)]
pub struct Conversation {
    /// Unique conversation identifier (monotonic counter, runtime-only).
    pub id: u64,
    /// Name of the agent this conversation is bound to.
    pub agent: String,
    /// Conversation history (the working context for the LLM).
    pub history: Vec<HistoryEntry>,
    /// Origin of this conversation (e.g. "user", "tg:12345").
    pub created_by: String,
    /// Conversation title (set by the `set_title` tool).
    pub title: String,
    /// Accumulated active time in seconds.
    pub uptime_secs: u64,
    /// When this conversation was loaded/created in this process.
    pub created_at: Instant,
    /// Persistent session identity, assigned by the repo. `None` until
    /// the first persistence call.
    pub handle: Option<SessionHandle>,
}

impl Conversation {
    /// Create a new conversation with an empty history.
    pub fn new(id: u64, agent: impl Into<String>, created_by: impl Into<String>) -> Self {
        Self {
            id,
            agent: agent.into(),
            history: Vec::new(),
            created_by: created_by.into(),
            title: String::new(),
            uptime_secs: 0,
            created_at: Instant::now(),
            handle: None,
        }
    }

    /// Build a [`ConversationMeta`] snapshot from this conversation's
    /// current state.
    pub fn meta(&self) -> ConversationMeta {
        ConversationMeta {
            agent: self.agent.clone(),
            created_by: self.created_by.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            title: self.title.clone(),
            uptime_secs: self.uptime_secs,
        }
    }
}

/// Sanitize a string into a filesystem-safe slug.
pub fn sender_slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

//! Persistence trait and domain types.
//!
//! [`Storage`] is the unified persistence backend — one trait, one
//! implementation per backend. Memory lives in its own `crabtalk-memory`
//! crate and is not part of this trait.

use crate::{
    AgentConfig, AgentEvent, AgentId, AgentStep, DaemonConfig, McpServerConfig, model::HistoryEntry,
};
use anyhow::Result;
use crabllm_core::Usage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Storage trait ───────────────────────────────────────────────────

/// Unified persistence backend.
///
/// All read/write operations for skills, sessions, and agents live
/// here. Implementations own their encoding and storage layout — the
/// trait speaks domain types only.
pub trait Storage: Send + Sync + 'static {
    // ── Skills (read-only — skills are discovered from disk, not
    //    created through the runtime) ───────────────────────────────

    /// List all available skills.
    fn list_skills(&self) -> Result<Vec<Skill>>;

    /// Load a skill by name. Returns `None` if not found.
    fn load_skill(&self, name: &str) -> Result<Option<Skill>>;

    // ── Sessions ───────────────────────────────────────────────────

    /// Create a new session. Returns an opaque handle.
    fn create_session(&self, agent: &str, created_by: &str) -> Result<SessionHandle>;

    /// Find the latest session for an (agent, created_by) identity.
    fn find_latest_session(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>>;

    /// Load a session's meta and working-context history.
    fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>>;

    /// List all sessions.
    fn list_sessions(&self) -> Result<Vec<SessionSummary>>;

    /// Append history entries to a session.
    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()>;

    /// Append trace event entries.
    fn append_session_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()>;

    /// Append a compact marker (archive boundary). `archive_name`
    /// references the `Archive`-kind entry in `memory` where the
    /// summary content actually lives. The marker only carries the
    /// pointer — session storage never sees the summary text.
    fn append_session_compact(&self, handle: &SessionHandle, archive_name: &str) -> Result<()>;

    /// Overwrite session metadata.
    fn update_session_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()>;

    /// Delete a session entirely.
    fn delete_session(&self, handle: &SessionHandle) -> Result<bool>;

    // ── Agents ─────────────────────────────────────────────────────

    /// List all persisted agent configs (with prompts loaded).
    fn list_agents(&self) -> Result<Vec<AgentConfig>>;

    /// Load a single agent by ULID.
    fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>>;

    /// Load a single agent by name.
    fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>>;

    /// Create or replace an agent config and prompt. `config.id` and
    /// `config.name` must both be set — implementations bail otherwise
    /// (otherwise the prompt becomes an orphan, unreachable by name or
    /// by listing).
    fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()>;

    /// Delete an agent by ULID. Returns `true` if it existed.
    fn delete_agent(&self, id: &AgentId) -> Result<bool>;

    /// Rename an agent. The ULID stays stable.
    fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool>;

    // ── Config ──────────────────────────────────────────────────────

    /// Load the daemon configuration (`config.toml`).
    fn load_config(&self) -> Result<DaemonConfig>;

    /// Overwrite the daemon configuration.
    fn save_config(&self, config: &DaemonConfig) -> Result<()>;

    /// Create the initial config directory structure and seed the
    /// default `crab` agent if no agent is stored yet.
    ///
    /// `default_model` is the model assigned to the seeded crab agent.
    /// Callers pick it from the configured providers; an empty string
    /// here would produce an unusable agent, so callers must ensure a
    /// provider is configured first.
    fn scaffold(&self, default_model: &str) -> Result<()>;

    // ── MCP servers ────────────────────────────────────────────────

    /// List all persisted MCP server configs, keyed by name.
    fn list_mcps(&self) -> Result<BTreeMap<String, McpServerConfig>>;

    /// Load a single MCP server by name.
    fn load_mcp(&self, name: &str) -> Result<Option<McpServerConfig>>;

    /// Create or replace an MCP server config. Keyed by `config.name`.
    fn upsert_mcp(&self, config: &McpServerConfig) -> Result<()>;

    /// Delete an MCP server by name. `true` if it existed.
    fn delete_mcp(&self, name: &str) -> Result<bool>;
}

/// Reject names that won't survive serialization as a TOML table key.
/// Used by MCP and agent CRUD to keep `local/settings.toml` from
/// silently aliasing entries (e.g. `mcp."foo.bar"` round-trips today
/// but a hand-edit dropping the quotes would corrupt the file).
pub fn validate_table_name(kind: &str, name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("{kind}: name must not be empty");
    }
    if name
        .chars()
        .any(|c| matches!(c, '.' | '[' | ']' | '"') || c.is_control())
    {
        anyhow::bail!(
            "{kind}: name '{name}' must not contain '.', '[', ']', '\"', or control chars"
        );
    }
    Ok(())
}

// ── Sessions ────────────────────────────────────────────────────────

/// Opaque handle identifying a persisted session. Created by the repo
/// on `create`, returned by `find_latest`. Callers pass it back to
/// append/load methods without interpreting the inner value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionHandle(String);

impl SessionHandle {
    /// Construct a handle from a repo-assigned identifier.
    pub fn new(slug: impl Into<String>) -> Self {
        Self(slug.into())
    }

    /// The raw identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Snapshot returned by [`Storage::load_session`] — meta +
/// working-context history, already replayed past the last compact
/// marker.
pub struct SessionSnapshot {
    pub meta: ConversationMeta,
    pub history: Vec<HistoryEntry>,
    /// Name of the `Archive`-kind memory entry whose content represents
    /// the compacted prefix of this session, if any. Callers that want
    /// the full resumed context resolve this against `memory` and
    /// prepend the entry's content to `history`.
    pub archive: Option<String>,
}

/// Summary returned by [`Storage::list_sessions`] for enumeration.
pub struct SessionSummary {
    pub handle: SessionHandle,
    pub meta: ConversationMeta,
}

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
    /// Topic this conversation belongs to, if any. `None` means the
    /// conversation is a tmp chat and should not have been persisted —
    /// only topic-bound conversations reach the Storage layer.
    #[serde(default)]
    pub topic: Option<String>,
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

/// Sanitize a string into a filesystem-safe slug for session naming.
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

// ── Skills ──────────────────────────────────────────────────────────

/// A named unit of agent behavior (agentskills.io format).
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub allowed_tools: Vec<String>,
    pub body: String,
}

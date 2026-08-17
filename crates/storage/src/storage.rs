//! Persistence trait and domain types.
//!
//! [`Storage`] is the unified persistence backend — one trait, one
//! implementation per backend. Memory lives in its own `crabtalk-memory`
//! crate and is not part of this trait.

use crate::{
    AgentConfig, AgentId, Config,
    session::history::HistoryEntry,
    session::{SearchOptions, SessionHit},
};
use anyhow::Result;
use crabllm_core::Usage;
use serde::{Deserialize, Serialize};
pub use skill::Skill;
use std::future::Future;

/// Unified persistence backend.
pub trait Storage: Send + Sync + 'static {
    // ── Skills (read-only — skills are discovered from disk, not
    //    created through the runtime) ───────────────────────────────

    /// List all available skills.
    fn list_skills(&self) -> impl Future<Output = Result<Vec<Skill>>> + Send;

    /// Load a skill by name. Returns `None` if not found.
    fn load_skill(&self, name: &str) -> impl Future<Output = Result<Option<Skill>>> + Send;

    // ── Sessions ───────────────────────────────────────────────────

    /// Create a new session. Returns an opaque handle.
    fn create_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> impl Future<Output = Result<SessionHandle>> + Send;

    /// Find the latest session for an (agent, created_by) identity.
    fn find_latest_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> impl Future<Output = Result<Option<SessionHandle>>> + Send;

    /// Load a session's meta and working-context history.
    fn load_session(
        &self,
        handle: &SessionHandle,
    ) -> impl Future<Output = Result<Option<SessionSnapshot>>> + Send;

    /// List all sessions.
    fn list_sessions(&self) -> impl Future<Output = Result<Vec<SessionSummary>>> + Send;

    /// Append history entries to a session.
    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Append trace event entries.
    fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Append a compact marker (archive boundary). `archive_name`
    /// references the `Archive`-kind entry in `memory` where the
    /// summary content actually lives. The marker only carries the
    /// pointer — session storage never sees the summary text.
    fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive_name: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Rewind a session's post-compact history to its first `keep`
    /// entries, dropping the rest — a conversation edit. The compacted
    /// prefix (stored as the `archive` pointer, never a message row) is
    /// preserved; only the live tail is trimmed. `keep` counts entries
    /// after the last compaction, matching `SessionSnapshot::history`.
    fn truncate_session_messages(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Overwrite session metadata.
    fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &ConversationMeta,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Delete a session entirely.
    fn delete_session(&self, handle: &SessionHandle) -> impl Future<Output = Result<bool>> + Send;

    // ── Session search ─────────────────────────────────────────────

    /// Search conversation messages. Best hit per session, up to
    /// `opts.limit`, each with a windowed excerpt.
    ///
    /// How a backend makes this fast is its own business — a resident
    /// index, FTS5, a GIN index. The trait only says it is a query, which
    /// means it awaits and it can fail.
    fn search_sessions(
        &self,
        query: &str,
        opts: &SearchOptions,
    ) -> impl Future<Output = Result<Vec<SessionHit>>> + Send;

    // ── Agents ─────────────────────────────────────────────────────

    /// List all persisted agent configs (with prompts loaded).
    fn list_agents(&self) -> impl Future<Output = Result<Vec<AgentConfig>>> + Send;

    /// Load a single agent by ULID.
    fn load_agent(&self, id: &AgentId) -> impl Future<Output = Result<Option<AgentConfig>>> + Send;

    /// Load a single agent by name.
    fn load_agent_by_name(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<AgentConfig>>> + Send;

    /// Create or replace an agent config. `config.id` and `config.name` must
    /// both be set — implementations bail otherwise, since an agent
    /// reachable by neither name nor listing is an orphan.
    fn upsert_agent(&self, config: &AgentConfig) -> impl Future<Output = Result<()>> + Send;

    /// Delete an agent by ULID. Returns `true` if it existed.
    fn delete_agent(&self, id: &AgentId) -> impl Future<Output = Result<bool>> + Send;

    /// Rename an agent. The ULID stays stable.
    fn rename_agent(
        &self,
        id: &AgentId,
        new_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send;

    // ── Config ──────────────────────────────────────────────────────

    /// Load the daemon configuration (`config.toml`).
    fn load_config(&self) -> impl Future<Output = Result<Config>> + Send;

    /// Overwrite the daemon configuration.
    fn save_config(&self, config: &Config) -> impl Future<Output = Result<()>> + Send;

    /// Create the initial config directory structure and seed the
    /// default `crab` agent if no agent is stored yet.
    fn scaffold(&self, default_model: &str) -> impl Future<Output = Result<()>> + Send;
}

/// Reject names that won't survive serialization as a TOML table key.
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

/// Opaque handle identifying a persisted session.
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
    pub updated_at: String,
    #[serde(default)]
    pub message_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A trace entry persisted alongside messages.
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

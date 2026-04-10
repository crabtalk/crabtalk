//! Unified storage trait.
//!
//! [`Storage`] is the single persistence backend for the runtime. It
//! replaces the four sub-traits (`MemoryRepo`, `SkillRepo`, `SessionRepo`,
//! `AgentRepo`) and their `Repos` composite with one flat interface.
//! One implementation per backend — filesystem, in-memory, database.

use crate::{
    AgentConfig, AgentId, ManifestConfig,
    model::HistoryEntry,
    repos::{MemoryEntry, SessionHandle, SessionSnapshot, SessionSummary, Skill},
    runtime::conversation::{ArchiveSegment, ConversationMeta, EventLine},
};
use anyhow::Result;

/// Unified persistence backend.
///
/// All read/write operations for memory, skills, sessions, and agents
/// live here. Implementations own their encoding and storage layout —
/// the trait speaks domain types only.
pub trait Storage: Send + Sync + 'static {
    // ── Memory ─────────────────────────────────────────────────────

    /// List all memory entries.
    fn list_memories(&self) -> Result<Vec<MemoryEntry>>;

    /// Load a memory entry by name.
    fn load_memory(&self, name: &str) -> Result<Option<MemoryEntry>>;

    /// Create or replace a memory entry.
    fn save_memory(&self, entry: &MemoryEntry) -> Result<()>;

    /// Delete a memory entry by name. Returns `true` if it existed.
    fn delete_memory(&self, name: &str) -> Result<bool>;

    /// Load the curated MEMORY.md index content.
    fn load_memory_index(&self) -> Result<Option<String>>;

    /// Overwrite the MEMORY.md index content.
    fn save_memory_index(&self, content: &str) -> Result<()>;

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

    /// Load all archive segments (compact markers) for a session.
    fn load_session_archives(&self, handle: &SessionHandle) -> Result<Vec<ArchiveSegment>>;

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

    /// Append a compact marker (archive boundary).
    fn append_session_compact(&self, handle: &SessionHandle, summary: &str) -> Result<()>;

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

    /// Create or replace an agent config and prompt.
    fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()>;

    /// Delete an agent by ULID. Returns `true` if it existed.
    fn delete_agent(&self, id: &AgentId) -> Result<bool>;

    /// Rename an agent. The ULID stays stable.
    fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool>;

    // ── Manifest ───────────────────────────────────────────────────

    /// Load the local manifest (`local/CrabTalk.toml`).
    /// Returns a default (empty) manifest if the file doesn't exist.
    fn load_local_manifest(&self) -> Result<ManifestConfig>;

    /// Overwrite the local manifest.
    fn save_local_manifest(&self, manifest: &ManifestConfig) -> Result<()>;

    /// Create the initial config directory structure if it doesn't exist.
    fn scaffold(&self) -> Result<()>;
}

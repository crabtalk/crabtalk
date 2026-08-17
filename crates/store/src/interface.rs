//! What the runtime programs against.
//!
//! Five interfaces over the two primitives. The runtime holds these and
//! never the data behind them: a working set hydrates the key it is
//! asked for and drops at the end of a run, so residency is the impl's
//! decision rather than a field on `Runtime`. That is what makes a
//! different deployment a different implementation instead of a rewrite.
//!
//! Nothing here returns a collection of bodies. Every listing is
//! identities or summaries, and the body is a second call for the one
//! thing the caller kept.
//!
//! Method names carry their subject — `load_agent`, not `load`. The
//! split exists so a consumer can bound on the one interface it needs;
//! it is not a namespace, and a type implementing all six would
//! otherwise make every call site ambiguous.

use crate::{
    AgentConfig, AgentId, HistoryEntry, MemoryEntry, Skill, SkillSummary,
    session::{EventLine, SearchOptions, SessionHandle, SessionHit, SessionMeta, SessionSnapshot},
};
use anyhow::Result;
use std::future::Future;

/// Everything the runtime needs from a store.
///
/// Blanket-implemented: satisfy the six and you satisfy this. It exists
/// so `Config::Storage` names one bound, not six.
pub trait Backend: Agents + Sessions + Memory + Skills + Harnesses + Send + Sync + 'static {}

impl<T> Backend for T where
    T: Agents + Sessions + Memory + Skills + Harnesses + Send + Sync + 'static
{
}

/// Persisted agents.
pub trait Agents: Send + Sync {
    fn load_agent(&self, id: &AgentId) -> impl Future<Output = Result<Option<AgentConfig>>> + Send;

    fn load_agent_by_name(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<AgentConfig>>> + Send;

    /// Every agent id. Ids, not configs — a listing that loaded each one
    /// would read every system prompt in the store to render names.
    fn agent_ids(&self) -> impl Future<Output = Result<Vec<AgentId>>> + Send;

    fn upsert_agent(&self, config: &AgentConfig) -> impl Future<Output = Result<()>> + Send;

    fn delete_agent(&self, id: &AgentId) -> impl Future<Output = Result<bool>> + Send;

    fn rename_agent(
        &self,
        id: &AgentId,
        new_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// The agent a surface talks to when it names none.
    ///
    /// Store state, not configuration: the daemon writes it, so it does
    /// not belong in a file a person hand-edits. `None` before scaffold,
    /// or if it points at an agent that has since been deleted.
    fn default_agent(&self) -> impl Future<Output = Result<Option<AgentId>>> + Send;

    fn set_default_agent(&self, id: &AgentId) -> impl Future<Output = Result<()>> + Send;
}

/// Persisted sessions and their message streams.
pub trait Sessions: Send + Sync {
    fn create_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> impl Future<Output = Result<SessionHandle>> + Send;

    fn find_latest_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> impl Future<Output = Result<Option<SessionHandle>>> + Send;

    fn load_session(
        &self,
        handle: &SessionHandle,
    ) -> impl Future<Output = Result<Option<SessionSnapshot>>> + Send;

    /// Handles and meta, served from the index without reading a
    /// transcript.
    fn list_sessions(
        &self,
    ) -> impl Future<Output = Result<Vec<(SessionHandle, SessionMeta)>>> + Send;

    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> impl Future<Output = Result<()>> + Send;

    fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Record a compaction boundary. `archive` names the memory entry
    /// holding the summary — the marker carries the pointer, never the
    /// text.
    fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive: &str,
    ) -> impl Future<Output = Result<()>> + Send;

    fn truncate_session_messages(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> impl Future<Output = Result<()>> + Send;

    fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &SessionMeta,
    ) -> impl Future<Output = Result<()>> + Send;

    fn delete_session(&self, handle: &SessionHandle) -> impl Future<Output = Result<bool>> + Send;

    fn delete_sessions_of(&self, agent: &AgentId) -> impl Future<Output = Result<usize>> + Send;

    fn search_sessions(
        &self,
        query: &str,
        opts: &SearchOptions,
    ) -> impl Future<Output = Result<Vec<SessionHit>>> + Send;
}

/// The agent's brain — named entries, searched by relevance.
pub trait Memory: Send + Sync {
    fn memory(&self, name: &str) -> impl Future<Output = Result<Option<MemoryEntry>>> + Send;

    fn memory_names(&self) -> impl Future<Output = Result<Vec<String>>> + Send;

    fn put_memory(&self, entry: &MemoryEntry) -> impl Future<Output = Result<()>> + Send;

    fn remove_memory(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Entry names ranked by relevance. The bodies are `memory` calls the
    /// caller makes for the ones it keeps.
    fn search_memory(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;
}

/// Installed skills.
pub trait Skills: Send + Sync {
    /// A page of identities. Never bodies — `Skill::body` is the whole
    /// markdown, so a listing that carried it would read the catalog.
    fn list_skills(
        &self,
        limit: usize,
        offset: usize,
    ) -> impl Future<Output = Result<Vec<SkillSummary>>> + Send;

    fn load_skill(&self, name: &str) -> impl Future<Output = Result<Option<Skill>>> + Send;

    /// Store a skill from its `SKILL.md`. The markdown is what is kept —
    /// it is the standard's own format, so it round-trips exactly and the
    /// name cannot disagree with the frontmatter it came from.
    fn put_skill(&self, markdown: &str) -> impl Future<Output = Result<SkillSummary>> + Send;

    fn remove_skill(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;
}

/// Harness images, addressed by digest.
///
/// The digest is the identity: an image already loaded under one is the
/// same sandbox, so residency needs no invalidation and two agents
/// declaring the same harness share one instantiation. `resolve_harness`
/// is the only mutable part — the name that currently points at a digest.
///
/// Shaped for extraction: berm will own this interface once it is split
/// out, and these three methods are what it needs.
pub trait Harnesses: Send + Sync {
    fn harness_image(&self, digest: &str) -> impl Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Store an image and return the digest it is keyed by.
    fn put_harness_image(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<String>> + Send;

    fn resolve_harness(&self, name: &str) -> impl Future<Output = Result<Option<String>>> + Send;
}

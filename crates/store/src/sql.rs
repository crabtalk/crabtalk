//! The SQL primitive — a derived index over [`KVStorage`](crate::KVStorage).
//!
//! Every row here points at a KV key. Ordering fields, FTS terms, set
//! membership — the things a lookup needs to *find* content, never the
//! content itself. Three properties follow, and they are why the two
//! stores can coexist without a transaction between them:
//!
//! - Writes need no atomicity across stores. Drop index rows first and
//!   content second: a crash orphans content that nothing can reach and
//!   a sweep can collect, rather than leaving a row pointing at nothing.
//! - The index is rebuildable. Scan the KV column, re-derive the rows.
//!   Corruption here is recoverable; corruption in KV is the real thing.
//! - A query that goes wrong leaks metadata, not content — reading a
//!   body still requires a KV key, and that key carries its tenant.
//!
//! The surface is a closed set of named queries. `query(sql: &str)` would
//! hand every caller the freedom this trait exists to withhold; SQL is an
//! implementation detail behind these methods, not the interface.

use crate::{
    AgentId, SkillSummary,
    session::{SearchOptions, SessionHit},
    session::{SessionHandle, SessionMeta},
};
use anyhow::Result;
use std::future::Future;

/// One searchable message.
///
/// `idx` is the message's absolute position in the session, not its
/// position in the batch: entries that must not be indexed leave gaps,
/// and a window or a truncation addresses positions.
#[derive(Debug, Clone)]
pub struct MessageDoc {
    pub idx: usize,
    pub role: String,
    pub body: String,
}

/// The index primitive.
pub trait SqlIndex: Send + Sync + 'static {
    // ── Agents ─────────────────────────────────────────────────────

    /// Record an agent's queryable identity. Idempotent.
    fn index_agent(&self, id: &AgentId, name: &str) -> impl Future<Output = Result<()>> + Send;

    fn unindex_agent(&self, id: &AgentId) -> impl Future<Output = Result<bool>> + Send;

    /// Every agent id, for enumeration. Ids only — the configs are in KV
    /// and a listing that loaded each one would read every prompt.
    fn agent_ids(&self) -> impl Future<Output = Result<Vec<AgentId>>> + Send;

    fn agent_id_by_name(&self, name: &str) -> impl Future<Output = Result<Option<AgentId>>> + Send;

    /// Point an existing agent's row at a new name. `false` if absent.
    fn rename_agent(
        &self,
        id: &AgentId,
        new_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send;

    // ── Sessions ───────────────────────────────────────────────────

    /// Upsert a session's row from its meta.
    fn index_session(
        &self,
        handle: &SessionHandle,
        meta: &SessionMeta,
    ) -> impl Future<Output = Result<()>> + Send;

    fn unindex_session(&self, handle: &SessionHandle) -> impl Future<Output = Result<bool>> + Send;

    /// The most recently updated session for an identity.
    fn latest_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> impl Future<Output = Result<Option<SessionHandle>>> + Send;

    /// Every session's handle and meta — the listing, served entirely
    /// from the index without touching KV.
    fn session_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<(SessionHandle, SessionMeta)>>> + Send;

    /// Handles belonging to an agent, for cascading a purge.
    fn session_handles_of(
        &self,
        agent: &AgentId,
    ) -> impl Future<Output = Result<Vec<SessionHandle>>> + Send;

    // ── Messages ───────────────────────────────────────────────────

    /// Index message text for search. Each doc carries its own position.
    fn index_messages(
        &self,
        handle: &SessionHandle,
        docs: &[MessageDoc],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Drop indexed messages at position `keep` and beyond. Used by a
    /// session edit; `keep = 0` clears the session.
    fn drop_messages_from(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Best hit per session, up to `opts.limit`.
    ///
    /// Hits come back with an empty `window`: the surrounding messages
    /// are content, and content is KV's. Filling it is the caller's
    /// second step, over keys this result hands it.
    fn search_messages(
        &self,
        query: &str,
        opts: &SearchOptions,
    ) -> impl Future<Output = Result<Vec<SessionHit>>> + Send;

    // ── Memory ─────────────────────────────────────────────────────

    fn index_memory(&self, name: &str, content: &str) -> impl Future<Output = Result<()>> + Send;

    fn unindex_memory(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// Entry names ranked by relevance. Names, not entries — the bodies
    /// are KV reads the caller makes only for what it keeps.
    fn search_memory(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;

    fn memory_names(&self) -> impl Future<Output = Result<Vec<String>>> + Send;

    // ── Skills ─────────────────────────────────────────────────────

    fn index_skill(&self, summary: &SkillSummary) -> impl Future<Output = Result<()>> + Send;

    fn unindex_skill(&self, name: &str) -> impl Future<Output = Result<bool>> + Send;

    /// A page of skill summaries. Never bodies: `Skill::body` is the
    /// whole markdown, and an enumeration that carried it would read
    /// every skill in the store to render a list of names.
    fn skill_summaries(
        &self,
        limit: usize,
        offset: usize,
    ) -> impl Future<Output = Result<Vec<SkillSummary>>> + Send;
}

/// A shared handle is an index. Mirrors the [`KVStorage`](crate::KVStorage)
/// impl on `Arc` so one open database can serve as both halves of a
/// [`Store`](crate::Store).
impl<T: SqlIndex> SqlIndex for std::sync::Arc<T> {
    fn index_agent(&self, id: &AgentId, name: &str) -> impl Future<Output = Result<()>> + Send {
        (**self).index_agent(id, name)
    }

    fn unindex_agent(&self, id: &AgentId) -> impl Future<Output = Result<bool>> + Send {
        (**self).unindex_agent(id)
    }

    fn agent_ids(&self) -> impl Future<Output = Result<Vec<AgentId>>> + Send {
        (**self).agent_ids()
    }

    fn agent_id_by_name(&self, name: &str) -> impl Future<Output = Result<Option<AgentId>>> + Send {
        (**self).agent_id_by_name(name)
    }

    fn rename_agent(
        &self,
        id: &AgentId,
        new_name: &str,
    ) -> impl Future<Output = Result<bool>> + Send {
        (**self).rename_agent(id, new_name)
    }

    fn index_session(
        &self,
        handle: &SessionHandle,
        meta: &SessionMeta,
    ) -> impl Future<Output = Result<()>> + Send {
        (**self).index_session(handle, meta)
    }

    fn unindex_session(&self, handle: &SessionHandle) -> impl Future<Output = Result<bool>> + Send {
        (**self).unindex_session(handle)
    }

    fn latest_session(
        &self,
        agent: &AgentId,
        created_by: &str,
    ) -> impl Future<Output = Result<Option<SessionHandle>>> + Send {
        (**self).latest_session(agent, created_by)
    }

    fn session_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<(SessionHandle, SessionMeta)>>> + Send {
        (**self).session_rows()
    }

    fn session_handles_of(
        &self,
        agent: &AgentId,
    ) -> impl Future<Output = Result<Vec<SessionHandle>>> + Send {
        (**self).session_handles_of(agent)
    }

    fn index_messages(
        &self,
        handle: &SessionHandle,
        docs: &[MessageDoc],
    ) -> impl Future<Output = Result<()>> + Send {
        (**self).index_messages(handle, docs)
    }

    fn drop_messages_from(
        &self,
        handle: &SessionHandle,
        keep: usize,
    ) -> impl Future<Output = Result<()>> + Send {
        (**self).drop_messages_from(handle, keep)
    }

    fn search_messages(
        &self,
        query: &str,
        opts: &SearchOptions,
    ) -> impl Future<Output = Result<Vec<SessionHit>>> + Send {
        (**self).search_messages(query, opts)
    }

    fn index_memory(&self, name: &str, content: &str) -> impl Future<Output = Result<()>> + Send {
        (**self).index_memory(name, content)
    }

    fn unindex_memory(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        (**self).unindex_memory(name)
    }

    fn search_memory(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<String>>> + Send {
        (**self).search_memory(query, limit)
    }

    fn memory_names(&self) -> impl Future<Output = Result<Vec<String>>> + Send {
        (**self).memory_names()
    }

    fn index_skill(&self, summary: &SkillSummary) -> impl Future<Output = Result<()>> + Send {
        (**self).index_skill(summary)
    }

    fn unindex_skill(&self, name: &str) -> impl Future<Output = Result<bool>> + Send {
        (**self).unindex_skill(name)
    }

    fn skill_summaries(
        &self,
        limit: usize,
        offset: usize,
    ) -> impl Future<Output = Result<Vec<SkillSummary>>> + Send {
        (**self).skill_summaries(limit, offset)
    }
}

//! The `Storage` impl.
//!
//! Every body forwards to the inherent method of the same name in the
//! domain modules. Inherent methods win resolution, so this is
//! forwarding, not recursion — deleting one turns its trait method into
//! an infinite loop rather than a compile error.

use crate::{skills, sqlite::SqliteStorage};
use anyhow::Result;
use wcore::{
    AgentConfig, AgentId, Config,
    model::HistoryEntry,
    storage::{
        ConversationMeta, EventLine, SessionHandle, SessionSnapshot, SessionSummary, Skill, Storage,
    },
};

impl Storage for SqliteStorage {
    // Skills are markdown on disk for every backend; only sessions,
    // agents, and config live in the database.
    async fn list_skills(&self) -> Result<Vec<Skill>> {
        skills::list(&self.skill_roots).await
    }

    async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        skills::load(&self.skill_roots, name).await
    }

    async fn create_session(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        self.create_session(agent, created_by).await
    }

    async fn find_latest_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        self.find_latest_session(agent, created_by).await
    }

    async fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        self.load_session(handle).await
    }

    async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        self.list_sessions().await
    }

    async fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        self.append_session_messages(handle, entries).await
    }

    async fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> Result<()> {
        self.append_session_events(handle, events).await
    }

    async fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive_name: &str,
    ) -> Result<()> {
        self.append_session_compact(handle, archive_name).await
    }

    async fn truncate_session_messages(&self, handle: &SessionHandle, keep: usize) -> Result<()> {
        self.truncate_session_messages(handle, keep).await
    }

    async fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &ConversationMeta,
    ) -> Result<()> {
        self.update_session_meta(handle, meta).await
    }

    async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        self.delete_session(handle).await
    }

    async fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        self.list_agents().await
    }

    async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        self.load_agent(id).await
    }

    async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        self.load_agent_by_name(name).await
    }

    async fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        self.upsert_agent(config, prompt).await
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        self.delete_agent(id).await
    }

    async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        self.rename_agent(id, new_name).await
    }

    async fn load_config(&self) -> Result<Config> {
        self.load_config().await
    }

    async fn save_config(&self, config: &Config) -> Result<()> {
        self.save_config(config).await
    }

    async fn scaffold(&self, default_model: &str) -> Result<()> {
        self.scaffold(default_model).await
    }
}

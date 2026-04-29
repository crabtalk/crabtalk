//! In-memory [`Storage`] implementation for tests.

use crate::{
    AgentConfig, AgentId, DaemonConfig,
    model::HistoryEntry,
    storage::{
        ConversationMeta, EventLine, SessionHandle, SessionSnapshot, SessionSummary, Skill, Storage,
    },
};
use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;

/// Per-session state in the in-memory backend.
#[derive(Clone)]
struct SessionState {
    meta: ConversationMeta,
    messages: Vec<HistoryEntry>,
    events: Vec<EventLine>,
    compacts: Vec<(String, String)>,
}

/// In-memory [`Storage`] for tests.
pub struct InMemoryStorage {
    skills: Mutex<Vec<Skill>>,
    sessions: Mutex<HashMap<String, SessionState>>,
    next_session_seq: Mutex<u32>,
    agents: Mutex<HashMap<String, (AgentConfig, String)>>,
    config: Mutex<DaemonConfig>,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self {
            skills: Mutex::new(Vec::new()),
            sessions: Mutex::new(HashMap::new()),
            next_session_seq: Mutex::new(0),
            agents: Mutex::new(HashMap::new()),
            config: Mutex::new(DaemonConfig::default()),
        }
    }
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_skills(skills: Vec<Skill>) -> Self {
        Self {
            skills: Mutex::new(skills),
            ..Self::default()
        }
    }
}

impl Storage for InMemoryStorage {
    // ── Skills ─────────────────────────────────────────────────────

    async fn list_skills(&self) -> Result<Vec<Skill>> {
        Ok(self.skills.lock().clone())
    }

    async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        Ok(self.skills.lock().iter().find(|s| s.name == name).cloned())
    }

    // ── Sessions ───────────────────────────────────────────────────

    async fn create_session(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        let mut seq = self.next_session_seq.lock();
        *seq += 1;
        let slug = format!("{}_{}", agent, seq);
        let state = SessionState {
            meta: {
                let now = chrono::Utc::now().to_rfc3339();
                ConversationMeta {
                    agent: agent.to_owned(),
                    created_by: created_by.to_owned(),
                    created_at: now.clone(),
                    title: String::new(),
                    updated_at: now,
                    message_count: 0,
                    summary: None,
                }
            },
            messages: Vec::new(),
            events: Vec::new(),
            compacts: Vec::new(),
        };
        self.sessions.lock().insert(slug.clone(), state);
        Ok(SessionHandle::new(slug))
    }

    async fn find_latest_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> Result<Option<SessionHandle>> {
        let sessions = self.sessions.lock();
        for (slug, state) in sessions.iter() {
            if state.meta.agent == agent && state.meta.created_by == created_by {
                return Ok(Some(SessionHandle::new(slug.clone())));
            }
        }
        Ok(None)
    }

    async fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let sessions = self.sessions.lock();
        let Some(state) = sessions.get(handle.as_str()) else {
            return Ok(None);
        };
        let archive = state.compacts.last().map(|(name, _)| name.clone());
        Ok(Some(SessionSnapshot {
            meta: state.meta.clone(),
            history: state.messages.clone(),
            archive,
        }))
    }

    async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let sessions = self.sessions.lock();
        Ok(sessions
            .iter()
            .map(|(slug, state)| SessionSummary {
                handle: SessionHandle::new(slug.clone()),
                meta: state.meta.clone(),
            })
            .collect())
    }

    async fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        let mut sessions = self.sessions.lock();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.messages.extend(entries.iter().cloned());
        }
        Ok(())
    }

    async fn append_session_events(
        &self,
        handle: &SessionHandle,
        events: &[EventLine],
    ) -> Result<()> {
        let mut sessions = self.sessions.lock();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.events.extend(events.iter().cloned());
        }
        Ok(())
    }

    async fn append_session_compact(
        &self,
        handle: &SessionHandle,
        archive_name: &str,
    ) -> Result<()> {
        let mut sessions = self.sessions.lock();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state
                .compacts
                .push((archive_name.to_owned(), chrono::Utc::now().to_rfc3339()));
            state.messages.clear();
        }
        Ok(())
    }

    async fn update_session_meta(
        &self,
        handle: &SessionHandle,
        meta: &ConversationMeta,
    ) -> Result<()> {
        let mut sessions = self.sessions.lock();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.meta = meta.clone();
        }
        Ok(())
    }

    async fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        Ok(self.sessions.lock().remove(handle.as_str()).is_some())
    }

    // ── Agents ─────────────────────────────────────────────────────

    async fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        Ok(self
            .agents
            .lock()
            .values()
            .map(|(c, prompt)| {
                let mut config = c.clone();
                config.system_prompt = prompt.clone();
                config
            })
            .collect())
    }

    async fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        let agents = self.agents.lock();
        for (config, prompt) in agents.values() {
            if config.id == *id {
                let mut c = config.clone();
                c.system_prompt = prompt.clone();
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    async fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let agents = self.agents.lock();
        if let Some((config, prompt)) = agents.get(name) {
            let mut c = config.clone();
            c.system_prompt = prompt.clone();
            Ok(Some(c))
        } else {
            Ok(None)
        }
    }

    async fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        if config.id.is_nil() {
            anyhow::bail!("cannot upsert agent with nil ID");
        }
        if config.name.is_empty() {
            anyhow::bail!("cannot upsert agent with empty name");
        }
        self.agents
            .lock()
            .insert(config.name.clone(), (config.clone(), prompt.to_owned()));
        Ok(())
    }

    async fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        let mut agents = self.agents.lock();
        let name = agents
            .iter()
            .find(|(_, (c, _))| c.id == *id)
            .map(|(n, _)| n.clone());
        if let Some(name) = name {
            agents.remove(&name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        let mut agents = self.agents.lock();
        let old_name = agents
            .iter()
            .find(|(_, (c, _))| c.id == *id)
            .map(|(n, _)| n.clone());
        if let Some(old_name) = old_name
            && let Some((mut config, prompt)) = agents.remove(&old_name)
        {
            config.name = new_name.to_owned();
            agents.insert(new_name.to_owned(), (config, prompt));
            return Ok(true);
        }
        Ok(false)
    }

    async fn load_config(&self) -> Result<DaemonConfig> {
        Ok(self.config.lock().clone())
    }

    async fn save_config(&self, config: &DaemonConfig) -> Result<()> {
        *self.config.lock() = config.clone();
        Ok(())
    }

    async fn scaffold(&self, _default_model: &str) -> Result<()> {
        Ok(())
    }
}

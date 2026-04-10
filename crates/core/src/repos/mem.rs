//! In-memory repository implementations for tests.
//!
//! Each repo is backed by `Mutex<HashMap<..>>` or `Mutex<Vec<..>>`.
//! `InMemoryRepos` bundles all four into a single [`Repos`]
//! implementation.

use crate::{
    AgentConfig, AgentId,
    model::HistoryEntry,
    repos::{
        AgentRepo, MemoryEntry, MemoryRepo, Repos, SessionHandle, SessionRepo, SessionSnapshot,
        SessionSummary, Skill, SkillRepo,
    },
    runtime::conversation::{ArchiveSegment, ConversationMeta, EventLine},
};
use anyhow::Result;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

// ── InMemoryMemoryRepo ──────────────────────────────────────────────

#[derive(Default)]
pub struct InMemoryMemoryRepo {
    entries: Mutex<HashMap<String, MemoryEntry>>,
    index: Mutex<Option<String>>,
}

impl InMemoryMemoryRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MemoryRepo for InMemoryMemoryRepo {
    fn list(&self) -> Result<Vec<MemoryEntry>> {
        Ok(self.entries.lock().unwrap().values().cloned().collect())
    }

    fn load(&self, name: &str) -> Result<Option<MemoryEntry>> {
        Ok(self.entries.lock().unwrap().get(name).cloned())
    }

    fn save(&self, entry: &MemoryEntry) -> Result<()> {
        self.entries
            .lock()
            .unwrap()
            .insert(entry.name.clone(), entry.clone());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<bool> {
        Ok(self.entries.lock().unwrap().remove(name).is_some())
    }

    fn load_index(&self) -> Result<Option<String>> {
        Ok(self.index.lock().unwrap().clone())
    }

    fn save_index(&self, content: &str) -> Result<()> {
        *self.index.lock().unwrap() = Some(content.to_owned());
        Ok(())
    }
}

// ── InMemorySkillRepo ───────────────────────────────────────────────

#[derive(Default)]
pub struct InMemorySkillRepo {
    skills: Mutex<Vec<Skill>>,
}

impl InMemorySkillRepo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_skills(skills: Vec<Skill>) -> Self {
        Self {
            skills: Mutex::new(skills),
        }
    }
}

impl SkillRepo for InMemorySkillRepo {
    fn list(&self) -> Result<Vec<Skill>> {
        Ok(self.skills.lock().unwrap().clone())
    }

    fn load(&self, name: &str) -> Result<Option<Skill>> {
        Ok(self
            .skills
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.name == name)
            .cloned())
    }
}

// ── InMemorySessionRepo ─────────────────────────────────────────────

/// Per-session state in the in-memory backend.
#[derive(Clone)]
struct SessionState {
    meta: ConversationMeta,
    messages: Vec<HistoryEntry>,
    events: Vec<EventLine>,
    compacts: Vec<(String, String)>, // (summary, archived_at)
}

#[derive(Default)]
pub struct InMemorySessionRepo {
    sessions: Mutex<HashMap<String, SessionState>>,
    next_seq: Mutex<u32>,
}

impl InMemorySessionRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SessionRepo for InMemorySessionRepo {
    fn create(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        let mut seq = self.next_seq.lock().unwrap();
        *seq += 1;
        let slug = format!("{}_{}", agent, seq);
        let state = SessionState {
            meta: ConversationMeta {
                agent: agent.to_owned(),
                created_by: created_by.to_owned(),
                created_at: chrono::Utc::now().to_rfc3339(),
                title: String::new(),
                uptime_secs: 0,
            },
            messages: Vec::new(),
            events: Vec::new(),
            compacts: Vec::new(),
        };
        self.sessions.lock().unwrap().insert(slug.clone(), state);
        Ok(SessionHandle::new(slug))
    }

    fn find_latest(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>> {
        let sessions = self.sessions.lock().unwrap();
        // Return the last matching session by insertion order (HashMap
        // doesn't guarantee this, but for tests it's fine).
        for (slug, state) in sessions.iter() {
            if state.meta.agent == agent && state.meta.created_by == created_by {
                return Ok(Some(SessionHandle::new(slug.clone())));
            }
        }
        Ok(None)
    }

    fn load(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let sessions = self.sessions.lock().unwrap();
        let Some(state) = sessions.get(handle.as_str()) else {
            return Ok(None);
        };
        // Replay from last compact forward.
        let history = if let Some((summary, _)) = state.compacts.last() {
            let mut h = vec![HistoryEntry::user(summary)];
            h.extend(state.messages.clone());
            h
        } else {
            state.messages.clone()
        };
        Ok(Some(SessionSnapshot {
            meta: state.meta.clone(),
            history,
        }))
    }

    fn load_archives(&self, handle: &SessionHandle) -> Result<Vec<ArchiveSegment>> {
        let sessions = self.sessions.lock().unwrap();
        let Some(state) = sessions.get(handle.as_str()) else {
            return Ok(Vec::new());
        };
        Ok(state
            .compacts
            .iter()
            .map(|(summary, archived_at)| ArchiveSegment {
                title: summary.chars().take(60).collect(),
                summary: summary.clone(),
                archived_at: archived_at.clone(),
            })
            .collect())
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let sessions = self.sessions.lock().unwrap();
        Ok(sessions
            .iter()
            .map(|(slug, state)| SessionSummary {
                handle: SessionHandle::new(slug.clone()),
                meta: state.meta.clone(),
            })
            .collect())
    }

    fn append_messages(&self, handle: &SessionHandle, entries: &[HistoryEntry]) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.messages.extend(entries.iter().cloned());
        }
        Ok(())
    }

    fn append_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.events.extend(events.iter().cloned());
        }
        Ok(())
    }

    fn append_compact(&self, handle: &SessionHandle, summary: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state
                .compacts
                .push((summary.to_owned(), chrono::Utc::now().to_rfc3339()));
            // Clear messages — the compact summary becomes the new base.
            state.messages.clear();
        }
        Ok(())
    }

    fn update_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.meta = meta.clone();
        }
        Ok(())
    }

    fn delete(&self, handle: &SessionHandle) -> Result<bool> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .remove(handle.as_str())
            .is_some())
    }
}

// ── InMemoryAgentRepo ───────────────────────────────────────────────

#[derive(Default)]
pub struct InMemoryAgentRepo {
    agents: Mutex<HashMap<String, (AgentConfig, String)>>, // name -> (config, prompt)
}

impl InMemoryAgentRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AgentRepo for InMemoryAgentRepo {
    fn list(&self) -> Result<Vec<AgentConfig>> {
        Ok(self
            .agents
            .lock()
            .unwrap()
            .values()
            .map(|(c, prompt)| {
                let mut config = c.clone();
                config.system_prompt = prompt.clone();
                config
            })
            .collect())
    }

    fn load(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        let agents = self.agents.lock().unwrap();
        for (config, prompt) in agents.values() {
            if config.id == *id {
                let mut c = config.clone();
                c.system_prompt = prompt.clone();
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    fn load_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let agents = self.agents.lock().unwrap();
        if let Some((config, prompt)) = agents.get(name) {
            let mut c = config.clone();
            c.system_prompt = prompt.clone();
            Ok(Some(c))
        } else {
            Ok(None)
        }
    }

    fn upsert(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        self.agents
            .lock()
            .unwrap()
            .insert(config.name.clone(), (config.clone(), prompt.to_owned()));
        Ok(())
    }

    fn delete(&self, id: &AgentId) -> Result<bool> {
        let mut agents = self.agents.lock().unwrap();
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

    fn rename(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        let mut agents = self.agents.lock().unwrap();
        let old_name = agents
            .iter()
            .find(|(_, (c, _))| c.id == *id)
            .map(|(n, _)| n.clone());
        if let Some(old_name) = old_name {
            if let Some((mut config, prompt)) = agents.remove(&old_name) {
                config.name = new_name.to_owned();
                agents.insert(new_name.to_owned(), (config, prompt));
                return Ok(true);
            }
        }
        Ok(false)
    }
}

// ── InMemoryRepos ───────────────────────────────────────────────────

/// Composite in-memory backend implementing [`Repos`].
pub struct InMemoryRepos {
    pub memory: Arc<InMemoryMemoryRepo>,
    pub skills: Arc<InMemorySkillRepo>,
    pub sessions: Arc<InMemorySessionRepo>,
    pub agents: Arc<InMemoryAgentRepo>,
}

impl Default for InMemoryRepos {
    fn default() -> Self {
        Self {
            memory: Arc::new(InMemoryMemoryRepo::new()),
            skills: Arc::new(InMemorySkillRepo::new()),
            sessions: Arc::new(InMemorySessionRepo::new()),
            agents: Arc::new(InMemoryAgentRepo::new()),
        }
    }
}

impl InMemoryRepos {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Repos for InMemoryRepos {
    type Memory = InMemoryMemoryRepo;
    type Skills = InMemorySkillRepo;
    type Sessions = InMemorySessionRepo;
    type Agents = InMemoryAgentRepo;

    fn memory(&self) -> &Arc<InMemoryMemoryRepo> {
        &self.memory
    }

    fn skills(&self) -> &Arc<InMemorySkillRepo> {
        &self.skills
    }

    fn sessions(&self) -> &Arc<InMemorySessionRepo> {
        &self.sessions
    }

    fn agents(&self) -> &Arc<InMemoryAgentRepo> {
        &self.agents
    }
}

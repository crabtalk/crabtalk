//! In-memory [`Storage`] implementation for tests.

use crate::{
    AgentConfig, AgentId, ManifestConfig, NodeConfig,
    model::HistoryEntry,
    repos::{MemoryEntry, SessionHandle, SessionSnapshot, SessionSummary, Skill, Storage},
    runtime::conversation::{ArchiveSegment, ConversationMeta, EventLine},
};
use anyhow::Result;
use std::{collections::HashMap, sync::Mutex};

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
    memories: Mutex<HashMap<String, MemoryEntry>>,
    memory_index: Mutex<Option<String>>,
    skills: Mutex<Vec<Skill>>,
    sessions: Mutex<HashMap<String, SessionState>>,
    next_session_seq: Mutex<u32>,
    agents: Mutex<HashMap<String, (AgentConfig, String)>>,
    manifest: Mutex<ManifestConfig>,
    config: Mutex<NodeConfig>,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self {
            memories: Mutex::new(HashMap::new()),
            memory_index: Mutex::new(None),
            skills: Mutex::new(Vec::new()),
            sessions: Mutex::new(HashMap::new()),
            next_session_seq: Mutex::new(0),
            agents: Mutex::new(HashMap::new()),
            manifest: Mutex::new(ManifestConfig::default()),
            config: Mutex::new(NodeConfig::default()),
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
    // ── Memory ─────────────────────────────────────────────────────

    fn list_memories(&self) -> Result<Vec<MemoryEntry>> {
        Ok(self.memories.lock().unwrap().values().cloned().collect())
    }

    fn load_memory(&self, name: &str) -> Result<Option<MemoryEntry>> {
        Ok(self.memories.lock().unwrap().get(name).cloned())
    }

    fn save_memory(&self, entry: &MemoryEntry) -> Result<()> {
        self.memories
            .lock()
            .unwrap()
            .insert(entry.name.clone(), entry.clone());
        Ok(())
    }

    fn delete_memory(&self, name: &str) -> Result<bool> {
        Ok(self.memories.lock().unwrap().remove(name).is_some())
    }

    fn load_memory_index(&self) -> Result<Option<String>> {
        Ok(self.memory_index.lock().unwrap().clone())
    }

    fn save_memory_index(&self, content: &str) -> Result<()> {
        *self.memory_index.lock().unwrap() = Some(content.to_owned());
        Ok(())
    }

    // ── Skills ─────────────────────────────────────────────────────

    fn list_skills(&self) -> Result<Vec<Skill>> {
        Ok(self.skills.lock().unwrap().clone())
    }

    fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        Ok(self
            .skills
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.name == name)
            .cloned())
    }

    // ── Sessions ───────────────────────────────────────────────────

    fn create_session(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        let mut seq = self.next_session_seq.lock().unwrap();
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

    fn find_latest_session(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>> {
        let sessions = self.sessions.lock().unwrap();
        for (slug, state) in sessions.iter() {
            if state.meta.agent == agent && state.meta.created_by == created_by {
                return Ok(Some(SessionHandle::new(slug.clone())));
            }
        }
        Ok(None)
    }

    fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        let sessions = self.sessions.lock().unwrap();
        let Some(state) = sessions.get(handle.as_str()) else {
            return Ok(None);
        };
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

    fn load_session_archives(&self, handle: &SessionHandle) -> Result<Vec<ArchiveSegment>> {
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

    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.messages.extend(entries.iter().cloned());
        }
        Ok(())
    }

    fn append_session_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.events.extend(events.iter().cloned());
        }
        Ok(())
    }

    fn append_session_compact(&self, handle: &SessionHandle, summary: &str) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state
                .compacts
                .push((summary.to_owned(), chrono::Utc::now().to_rfc3339()));
            state.messages.clear();
        }
        Ok(())
    }

    fn update_session_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(state) = sessions.get_mut(handle.as_str()) {
            state.meta = meta.clone();
        }
        Ok(())
    }

    fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .remove(handle.as_str())
            .is_some())
    }

    // ── Agents ─────────────────────────────────────────────────────

    fn list_agents(&self) -> Result<Vec<AgentConfig>> {
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

    fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
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

    fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        let agents = self.agents.lock().unwrap();
        if let Some((config, prompt)) = agents.get(name) {
            let mut c = config.clone();
            c.system_prompt = prompt.clone();
            Ok(Some(c))
        } else {
            Ok(None)
        }
    }

    fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        self.agents
            .lock()
            .unwrap()
            .insert(config.name.clone(), (config.clone(), prompt.to_owned()));
        Ok(())
    }

    fn delete_agent(&self, id: &AgentId) -> Result<bool> {
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

    fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        let mut agents = self.agents.lock().unwrap();
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

    // ── Manifest ───────────────────────────────────────────────────

    fn load_local_manifest(&self) -> Result<ManifestConfig> {
        Ok(self.manifest.lock().unwrap().clone())
    }

    fn save_local_manifest(&self, manifest: &ManifestConfig) -> Result<()> {
        *self.manifest.lock().unwrap() = manifest.clone();
        Ok(())
    }

    fn load_config(&self) -> Result<NodeConfig> {
        Ok(self.config.lock().unwrap().clone())
    }

    fn save_config(&self, config: &NodeConfig) -> Result<()> {
        *self.config.lock().unwrap() = config.clone();
        Ok(())
    }

    fn scaffold(&self) -> Result<()> {
        Ok(())
    }
}

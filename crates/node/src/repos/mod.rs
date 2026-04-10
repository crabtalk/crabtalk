//! Filesystem-backed repository implementations.
//!
//! [`NodeRepos`] bundles all four into a single [`Repos`]
//! implementation wired up by the daemon builder.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use wcore::repos::{Repos, Storage};

pub mod agents;
pub mod memory;
pub mod sessions;
pub mod skills;

pub use agents::FsAgentRepo;
pub use memory::FsMemoryRepo;
pub use sessions::FsSessionRepo;
pub use skills::FsSkillRepo;

/// Composite filesystem persistence backend.
pub struct NodeRepos {
    pub memory: Arc<FsMemoryRepo>,
    pub skills: Arc<FsSkillRepo>,
    pub sessions: Arc<FsSessionRepo>,
    pub agents: Arc<FsAgentRepo>,
}

impl Repos for NodeRepos {
    type Memory = FsMemoryRepo;
    type Skills = FsSkillRepo;
    type Sessions = FsSessionRepo;
    type Agents = FsAgentRepo;

    fn memory(&self) -> &Arc<FsMemoryRepo> {
        &self.memory
    }

    fn skills(&self) -> &Arc<FsSkillRepo> {
        &self.skills
    }

    fn sessions(&self) -> &Arc<FsSessionRepo> {
        &self.sessions
    }

    fn agents(&self) -> &Arc<FsAgentRepo> {
        &self.agents
    }
}

/// Bridge: delegates Storage methods to the individual sub-repos.
/// Temporary — replaced by FsStorage in a later phase.
impl Storage for NodeRepos {
    fn list_memories(&self) -> anyhow::Result<Vec<wcore::repos::MemoryEntry>> {
        wcore::repos::MemoryRepo::list(self.memory.as_ref())
    }
    fn load_memory(&self, name: &str) -> anyhow::Result<Option<wcore::repos::MemoryEntry>> {
        wcore::repos::MemoryRepo::load(self.memory.as_ref(), name)
    }
    fn save_memory(&self, entry: &wcore::repos::MemoryEntry) -> anyhow::Result<()> {
        wcore::repos::MemoryRepo::save(self.memory.as_ref(), entry)
    }
    fn delete_memory(&self, name: &str) -> anyhow::Result<bool> {
        wcore::repos::MemoryRepo::delete(self.memory.as_ref(), name)
    }
    fn load_memory_index(&self) -> anyhow::Result<Option<String>> {
        wcore::repos::MemoryRepo::load_index(self.memory.as_ref())
    }
    fn save_memory_index(&self, content: &str) -> anyhow::Result<()> {
        wcore::repos::MemoryRepo::save_index(self.memory.as_ref(), content)
    }

    fn list_skills(&self) -> anyhow::Result<Vec<wcore::repos::Skill>> {
        wcore::repos::SkillRepo::list(self.skills.as_ref())
    }
    fn load_skill(&self, name: &str) -> anyhow::Result<Option<wcore::repos::Skill>> {
        wcore::repos::SkillRepo::load(self.skills.as_ref(), name)
    }

    fn create_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> anyhow::Result<wcore::repos::SessionHandle> {
        wcore::repos::SessionRepo::create(self.sessions.as_ref(), agent, created_by)
    }
    fn find_latest_session(
        &self,
        agent: &str,
        created_by: &str,
    ) -> anyhow::Result<Option<wcore::repos::SessionHandle>> {
        wcore::repos::SessionRepo::find_latest(self.sessions.as_ref(), agent, created_by)
    }
    fn load_session(
        &self,
        handle: &wcore::repos::SessionHandle,
    ) -> anyhow::Result<Option<wcore::repos::SessionSnapshot>> {
        wcore::repos::SessionRepo::load(self.sessions.as_ref(), handle)
    }
    fn load_session_archives(
        &self,
        handle: &wcore::repos::SessionHandle,
    ) -> anyhow::Result<Vec<wcore::ArchiveSegment>> {
        wcore::repos::SessionRepo::load_archives(self.sessions.as_ref(), handle)
    }
    fn list_sessions(&self) -> anyhow::Result<Vec<wcore::repos::SessionSummary>> {
        wcore::repos::SessionRepo::list_sessions(self.sessions.as_ref())
    }
    fn append_session_messages(
        &self,
        handle: &wcore::repos::SessionHandle,
        entries: &[wcore::model::HistoryEntry],
    ) -> anyhow::Result<()> {
        wcore::repos::SessionRepo::append_messages(self.sessions.as_ref(), handle, entries)
    }
    fn append_session_events(
        &self,
        handle: &wcore::repos::SessionHandle,
        events: &[wcore::EventLine],
    ) -> anyhow::Result<()> {
        wcore::repos::SessionRepo::append_events(self.sessions.as_ref(), handle, events)
    }
    fn append_session_compact(
        &self,
        handle: &wcore::repos::SessionHandle,
        summary: &str,
    ) -> anyhow::Result<()> {
        wcore::repos::SessionRepo::append_compact(self.sessions.as_ref(), handle, summary)
    }
    fn update_session_meta(
        &self,
        handle: &wcore::repos::SessionHandle,
        meta: &wcore::ConversationMeta,
    ) -> anyhow::Result<()> {
        wcore::repos::SessionRepo::update_meta(self.sessions.as_ref(), handle, meta)
    }
    fn delete_session(&self, handle: &wcore::repos::SessionHandle) -> anyhow::Result<bool> {
        wcore::repos::SessionRepo::delete(self.sessions.as_ref(), handle)
    }

    fn list_agents(&self) -> anyhow::Result<Vec<wcore::AgentConfig>> {
        wcore::repos::AgentRepo::list(self.agents.as_ref())
    }
    fn load_agent(&self, id: &wcore::AgentId) -> anyhow::Result<Option<wcore::AgentConfig>> {
        wcore::repos::AgentRepo::load(self.agents.as_ref(), id)
    }
    fn load_agent_by_name(&self, name: &str) -> anyhow::Result<Option<wcore::AgentConfig>> {
        wcore::repos::AgentRepo::load_by_name(self.agents.as_ref(), name)
    }
    fn upsert_agent(&self, config: &wcore::AgentConfig, prompt: &str) -> anyhow::Result<()> {
        wcore::repos::AgentRepo::upsert(self.agents.as_ref(), config, prompt)
    }
    fn delete_agent(&self, id: &wcore::AgentId) -> anyhow::Result<bool> {
        wcore::repos::AgentRepo::delete(self.agents.as_ref(), id)
    }
    fn rename_agent(&self, id: &wcore::AgentId, new_name: &str) -> anyhow::Result<bool> {
        wcore::repos::AgentRepo::rename(self.agents.as_ref(), id, new_name)
    }
}

/// Atomic write: same-directory tmp file + rename.
pub fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut tmp_os = path.to_path_buf().into_os_string();
    tmp_os.push(format!(".tmp.{}.{}", std::process::id(), nanos));
    let tmp_path = PathBuf::from(tmp_os);
    fs::write(&tmp_path, data)?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}

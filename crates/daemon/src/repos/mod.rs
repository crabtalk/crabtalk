//! Filesystem-backed repository implementations.
//!
//! [`DaemonRepos`] bundles all four into a single [`Repos`]
//! implementation wired up by the daemon builder.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use wcore::repos::Repos;

pub mod agents;
pub mod memory;
pub mod sessions;
pub mod skills;

pub use agents::FsAgentRepo;
pub use memory::FsMemoryRepo;
pub use sessions::FsSessionRepo;
pub use skills::FsSkillRepo;

/// Composite filesystem persistence backend.
pub struct DaemonRepos {
    pub memory: Arc<FsMemoryRepo>,
    pub skills: Arc<FsSkillRepo>,
    pub sessions: Arc<FsSessionRepo>,
    pub agents: Arc<FsAgentRepo>,
}

impl Repos for DaemonRepos {
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

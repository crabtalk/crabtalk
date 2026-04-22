//! Filesystem-backed [`Storage`] implementation.
//!
//! `FsStorage` owns the directory layout. Each storage domain
//! (sessions, agents, mcps, skills, config) lives in its own
//! submodule as free functions taking `&FsStorage`; the trait impl
//! below is pure delegation. Settings file reads/writes are shared
//! between agents, mcps, and config, so they sit on the struct itself.

use anyhow::Result;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use wcore::{
    AgentConfig, AgentId, ConversationMeta, DaemonConfig, EventLine, McpServerConfig,
    model::HistoryEntry,
    storage::{SessionHandle, SessionSnapshot, SessionSummary, Skill, Storage},
};

mod agents;
mod config;
mod mcp;
mod scaffold;
mod sessions;
mod skills;

pub use scaffold::default_crab;

/// Header prepended to `local/settings.toml`. Tells humans not to edit
/// the file while the daemon is running and signals the file is
/// daemon-owned.
const SETTINGS_HEADER: &str = "\
# Managed by crabtalk daemon. Edits while the daemon is running are
# overwritten on the next write. Edits while the daemon is stopped are
# picked up on next reload.
#
# Source of truth for runtime-added MCPs and agents. Immutable
# install-time configuration (providers, task pool) lives in
# config.toml.

";

/// Filesystem persistence backend.
pub struct FsStorage {
    /// Config directory root (for agent prompt storage under `agents/<ulid>/`).
    pub(super) config_dir: PathBuf,
    /// Root for session directories.
    pub(super) sessions_root: PathBuf,
    /// Ordered skill roots to scan (local first, then packages).
    pub(super) skill_roots: Vec<PathBuf>,
    /// Per-session step counters, recovered from disk on first access.
    pub(super) session_counters: Mutex<HashMap<String, u64>>,
}

impl FsStorage {
    pub fn new(config_dir: PathBuf, sessions_root: PathBuf, skill_roots: Vec<PathBuf>) -> Self {
        Self {
            config_dir,
            sessions_root,
            skill_roots,
            session_counters: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn settings_path(&self) -> PathBuf {
        self.config_dir.join(wcore::paths::SETTINGS_FILE)
    }

    /// Read and parse the settings file. Re-parsed on every call —
    /// settings are tiny and writes are rare, so a cache would be
    /// premature. Don't add one without measuring.
    pub(super) fn read_settings(&self) -> Result<SettingsFile> {
        let path = self.settings_path();
        match fs::read_to_string(&path) {
            Ok(content) => Ok(toml::from_str(&content)?),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(SettingsFile::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub(super) fn write_settings(&self, file: &SettingsFile) -> Result<()> {
        let path = self.settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(file)?;
        let mut content = String::with_capacity(SETTINGS_HEADER.len() + body.len());
        content.push_str(SETTINGS_HEADER);
        content.push_str(&body);
        atomic_write(&path, content.as_bytes())
    }
}

/// Atomic write: same-directory tmp file + rename. Shared by every
/// submodule that touches disk; lives here so the import path is
/// uniform (`super::atomic_write`).
pub(super) fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
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

/// On-disk shape of `local/settings.toml`. Holds runtime-added records:
///   - `[mcps.<name>]` — MCP server registrations
///   - `[agents.<name>]` — full agent definitions (model, members, …)
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct SettingsFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) mcps: BTreeMap<String, McpServerConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) agents: BTreeMap<String, AgentConfig>,
}

impl Storage for FsStorage {
    fn list_skills(&self) -> Result<Vec<Skill>> {
        skills::list_skills(self)
    }

    fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        skills::load_skill(self, name)
    }

    fn create_session(&self, agent: &str, created_by: &str) -> Result<SessionHandle> {
        sessions::create_session(self, agent, created_by)
    }

    fn find_latest_session(&self, agent: &str, created_by: &str) -> Result<Option<SessionHandle>> {
        sessions::find_latest_session(self, agent, created_by)
    }

    fn load_session(&self, handle: &SessionHandle) -> Result<Option<SessionSnapshot>> {
        sessions::load_session(self, handle)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        sessions::list_sessions(self)
    }

    fn append_session_messages(
        &self,
        handle: &SessionHandle,
        entries: &[HistoryEntry],
    ) -> Result<()> {
        sessions::append_session_messages(self, handle, entries)
    }

    fn append_session_events(&self, handle: &SessionHandle, events: &[EventLine]) -> Result<()> {
        sessions::append_session_events(self, handle, events)
    }

    fn append_session_compact(&self, handle: &SessionHandle, archive_name: &str) -> Result<()> {
        sessions::append_session_compact(self, handle, archive_name)
    }

    fn update_session_meta(&self, handle: &SessionHandle, meta: &ConversationMeta) -> Result<()> {
        sessions::update_session_meta(self, handle, meta)
    }

    fn delete_session(&self, handle: &SessionHandle) -> Result<bool> {
        sessions::delete_session(self, handle)
    }

    fn list_agents(&self) -> Result<Vec<AgentConfig>> {
        agents::list_agents(self)
    }

    fn load_agent(&self, id: &AgentId) -> Result<Option<AgentConfig>> {
        agents::load_agent(self, id)
    }

    fn load_agent_by_name(&self, name: &str) -> Result<Option<AgentConfig>> {
        agents::load_agent_by_name(self, name)
    }

    fn upsert_agent(&self, config: &AgentConfig, prompt: &str) -> Result<()> {
        agents::upsert_agent(self, config, prompt)
    }

    fn delete_agent(&self, id: &AgentId) -> Result<bool> {
        agents::delete_agent(self, id)
    }

    fn rename_agent(&self, id: &AgentId, new_name: &str) -> Result<bool> {
        agents::rename_agent(self, id, new_name)
    }

    fn load_config(&self) -> Result<DaemonConfig> {
        config::load_config(self)
    }

    fn save_config(&self, config: &DaemonConfig) -> Result<()> {
        config::save_config(self, config)
    }

    fn scaffold(&self) -> Result<()> {
        scaffold::scaffold(self)
    }

    fn list_mcps(&self) -> Result<BTreeMap<String, McpServerConfig>> {
        mcp::list_mcps(self)
    }

    fn load_mcp(&self, name: &str) -> Result<Option<McpServerConfig>> {
        mcp::load_mcp(self, name)
    }

    fn upsert_mcp(&self, config: &McpServerConfig) -> Result<()> {
        mcp::upsert_mcp(self, config)
    }

    fn delete_mcp(&self, name: &str) -> Result<bool> {
        mcp::delete_mcp(self, name)
    }
}

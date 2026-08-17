//! Filesystem-backed [`Storage`] implementation.
//!
//! `FsStorage` owns the directory layout. Each storage domain
//! (sessions, agents, skills, config, scaffold) lives in its own
//! submodule as a separate `impl FsStorage` block; the trait impl
//! below forwards to those inherent methods, which shadow it by the
//! same name. Settings file reads/writes are shared between agents
//! and config, so they sit on the struct itself.

use anyhow::Result;
pub use loader::{DEFAULT_CONFIG, DEFAULT_SETTINGS, scaffold_config_dir};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::fs;
use wcore::{
    AgentConfig, AgentId, Config, ConversationMeta, EventLine,
    model::HistoryEntry,
    storage::{SessionHandle, SessionSnapshot, SessionSummary, Skill, Storage},
};

mod agents;
mod config;
mod loader;
mod scaffold;
mod sessions;
mod skills;

/// Header prepended to `local/settings.toml` on every write — same
/// template as the one scaffolded on first-run (`DEFAULT_SETTINGS`).
/// Tells humans not to edit the file while the daemon is running and
/// documents the allowed sections.
pub(super) const SETTINGS_HEADER: &str = DEFAULT_SETTINGS;

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
    pub(super) async fn read_settings(&self) -> Result<SettingsFile> {
        let path = self.settings_path();
        match fs::read_to_string(&path).await {
            Ok(content) => Ok(toml::from_str(&content)?),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(SettingsFile::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub(super) async fn write_settings(&self, file: &SettingsFile) -> Result<()> {
        let path = self.settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let body = toml::to_string_pretty(file)?;
        let mut content = String::with_capacity(SETTINGS_HEADER.len() + body.len());
        content.push_str(SETTINGS_HEADER);
        content.push_str(&body);
        atomic_write(&path, content.as_bytes()).await
    }
}

/// Atomic write: same-directory tmp file + rename. Shared by every
/// submodule that touches disk; lives here so the import path is
/// uniform (`super::atomic_write`).
pub(super) async fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut tmp_os = path.to_path_buf().into_os_string();
    tmp_os.push(format!(".tmp.{}.{}", std::process::id(), nanos));
    let tmp_path = PathBuf::from(tmp_os);
    fs::write(&tmp_path, data).await?;
    if let Err(e) = fs::rename(&tmp_path, path).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(e.into());
    }
    Ok(())
}

/// On-disk shape of `local/settings.toml`. Holds runtime-added records:
///   - `[agents.<name>]` — full agent definitions (model, MCPs, skills, …)
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct SettingsFile {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) agents: BTreeMap<String, AgentConfig>,
}

/// Every body here calls the inherent method of the same name, defined
/// in the domain module. Inherent methods win resolution, so this is
/// forwarding, not recursion — deleting one turns its trait method into
/// an infinite loop rather than a compile error.
impl Storage for FsStorage {
    async fn list_skills(&self) -> Result<Vec<Skill>> {
        self.list_skills().await
    }

    async fn load_skill(&self, name: &str) -> Result<Option<Skill>> {
        self.load_skill(name).await
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

    async fn upsert_agent(&self, config: &AgentConfig) -> Result<()> {
        self.upsert_agent(config).await
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

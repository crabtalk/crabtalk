//! OS tools — bash, read, edit — as a Hook implementation.

use crate::daemon::ConversationCwds;
use bash::Bash;
use edit::Edit;
use parking_lot::{Mutex, RwLock};
use read::Read;
use runtime::Hook;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write,
    path::PathBuf,
    sync::Arc,
};
use wcore::{AgentConfig, BashConfig, ToolDispatch, ToolFuture, agent::AsTool};

mod bash;
mod edit;
mod read;

/// Per-conversation set of files that have been read (shared with DelegateHook
/// for cleanup when delegated conversations close).
pub type ReadFiles = Arc<Mutex<HashMap<u64, HashSet<PathBuf>>>>;

/// Maximum file size in bytes before refusing to read (50 MB).
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Build an `<environment>` XML block with OS info.
fn environment_block() -> String {
    let mut buf = String::from("\n\n<environment>\n");
    let _ = writeln!(buf, "os: {}", std::env::consts::OS);
    buf.push_str("</environment>");
    buf
}

/// OS tools subsystem: bash, read, edit.
///
/// Owns the base working directory and per-conversation CWD overrides.
/// Injects the working directory environment block before each run.
pub struct OsHook {
    cwd: PathBuf,
    conversation_cwds: ConversationCwds,
    /// Files read per conversation — edit requires a prior read.
    read_files: ReadFiles,
    /// Per-agent bash policy cache, populated from `on_register_agent`.
    /// Avoids an async storage roundtrip from the sync `dispatch` path.
    configs: RwLock<BTreeMap<String, BashConfig>>,
}

impl OsHook {
    pub fn new(cwd: PathBuf, conversation_cwds: ConversationCwds, read_files: ReadFiles) -> Self {
        Self {
            cwd,
            conversation_cwds,
            read_files,
            configs: RwLock::new(BTreeMap::new()),
        }
    }

    /// Look up an agent's bash configuration. Falls back to
    /// [`BashConfig::default`] for unknown agents — the dispatcher
    /// rejects unknown agents anyway, so this only matters for
    /// `scoped_tools`.
    fn bash_config(&self, agent: &str) -> BashConfig {
        self.configs.read().get(agent).cloned().unwrap_or_default()
    }

    /// Effective `disabled` flag for an agent.
    pub fn bash_disabled(&self, agent: &str) -> bool {
        self.bash_config(agent).disabled
    }

    /// Effective deny list for an agent.
    pub fn bash_deny(&self, agent: &str) -> Vec<String> {
        self.bash_config(agent).deny
    }

    /// Per-conversation CWD overrides.
    pub fn conversation_cwds(&self) -> &ConversationCwds {
        &self.conversation_cwds
    }

    /// Record that a file was read in a conversation.
    fn record_read(&self, conversation_id: u64, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.read_files
            .lock()
            .entry(conversation_id)
            .or_default()
            .insert(path);
    }

    /// Check whether a file was read in a conversation.
    fn was_read(&self, conversation_id: Option<u64>, path: &std::path::Path) -> bool {
        let Some(id) = conversation_id else {
            return false;
        };
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.read_files
            .lock()
            .get(&id)
            .is_some_and(|set| set.contains(&path))
    }

    fn effective_cwd(&self, conversation_id: Option<u64>) -> PathBuf {
        if let Some(id) = conversation_id
            && let Ok(map) = self.conversation_cwds.try_lock()
            && let Some(cwd) = map.get(&id)
        {
            return cwd.clone();
        }
        self.cwd.clone()
    }
}

impl Hook for OsHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        // Always advertise bash at the global level; per-agent gating
        // happens in `scoped_tools`. Skipping the schema here would make
        // the tool invisible to every agent regardless of overrides.
        vec![Bash::as_tool(), Read::as_tool(), Edit::as_tool()]
    }

    fn scoped_tools(&self, config: &AgentConfig) -> (Vec<String>, Option<String>) {
        let mut tools = vec![Read::as_tool().function.name, Edit::as_tool().function.name];
        let bash = &config.hooks.bash;
        if !bash.disabled {
            tools.insert(0, Bash::as_tool().function.name);
        }
        let policy = bash::config::prompt_block(bash);
        (tools, policy)
    }

    fn system_prompt(&self) -> Option<String> {
        Some(environment_block())
    }

    fn on_register_agent(&self, name: &str, config: &AgentConfig) {
        self.configs
            .write()
            .insert(name.to_owned(), config.hooks.bash.clone());
    }

    fn on_unregister_agent(&self, name: &str) {
        self.configs.write().remove(name);
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        match name {
            "bash" if !self.bash_disabled(&call.agent) => Some(Box::pin(self.handle_bash(call))),
            "read" => Some(Box::pin(self.handle_read(call))),
            "edit" => Some(Box::pin(self.handle_edit(call))),
            _ => None,
        }
    }
}

//! OS tools — bash, read, edit — as a Hook implementation.

use bash::Bash;
use edit::Edit;
use parking_lot::Mutex;
use read::Read;
use runtime::Hook;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Mutex as AsyncMutex;
use wcore::{ToolDispatch, ToolFuture, agent::AsTool};

mod bash;
mod edit;
mod read;

/// Per-conversation working directory overrides.
///
/// Shared between the host's `Env::effective_cwd` implementation, `OsHook`
/// (for tool dispatch), and any subsystem that needs to mutate per-call
/// cwd (e.g. delegated child conversations).
pub type ConversationCwds = Arc<AsyncMutex<HashMap<u64, PathBuf>>>;

/// Per-conversation set of files that have been read.
///
/// Edit refuses to operate on a file that has not been read in the current
/// conversation — the model must observe the file before mutating it.
/// Shared with delegate-style subsystems that need to clean up entries
/// when delegated conversations close.
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
}

impl OsHook {
    pub fn new(cwd: PathBuf, conversation_cwds: ConversationCwds, read_files: ReadFiles) -> Self {
        Self {
            cwd,
            conversation_cwds,
            read_files,
        }
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

/// The OS tool set's static schemas — bash, read, edit. Hosts that
/// advertise these tools (daemon-side `ClientToolHook` for forwarding,
/// TUI-side local execution) call this to avoid duplicating the schema
/// derivations.
pub fn schemas() -> Vec<wcore::model::Tool> {
    vec![Bash::as_tool(), Read::as_tool(), Edit::as_tool()]
}

/// Names of the OS tool set. Used by `ClientToolHook` to recognise
/// dispatches that should be forwarded to the client.
pub fn names() -> Vec<String> {
    schemas().into_iter().map(|t| t.function.name).collect()
}

impl Hook for OsHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        schemas()
    }

    fn system_prompt(&self) -> Option<String> {
        Some(environment_block())
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        match name {
            "bash" => Some(Box::pin(self.handle_bash(call))),
            "read" => Some(Box::pin(self.handle_read(call))),
            "edit" => Some(Box::pin(self.handle_edit(call))),
            _ => None,
        }
    }
}

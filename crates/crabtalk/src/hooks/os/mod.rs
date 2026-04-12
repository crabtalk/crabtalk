//! OS tools — bash, read, edit — as a Hook implementation.

use crate::node::hook::ConversationCwds;
use bash::Bash;
pub use bash::BashConfig;
use edit::Edit;
use read::Read;
use runtime::Hook;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool, model::HistoryEntry};

mod bash;
mod edit;
mod read;

/// Per-conversation set of files that have been read (shared with DelegateHook
/// for cleanup when delegated conversations close).
pub type ReadFiles = Arc<Mutex<HashMap<u64, HashSet<PathBuf>>>>;

/// A bash approval request sent to the app layer.
pub struct ApprovalRequest {
    /// The command that was denied.
    pub command: String,
    /// Why it was denied (e.g. "not in allow list" or "denied by policy").
    pub reason: String,
    /// Send `true` to approve, `false` to deny.
    pub reply: oneshot::Sender<bool>,
}

/// Sender half of the approval channel.
pub type ApprovalTx = mpsc::Sender<ApprovalRequest>;

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
    /// Bash command policy.
    bash_config: BashConfig,
    /// Approval channel for interactive bash policy prompts.
    approval_tx: ApprovalTx,
}

impl OsHook {
    pub fn new(
        cwd: PathBuf,
        conversation_cwds: ConversationCwds,
        read_files: ReadFiles,
        bash_config: BashConfig,
        approval_tx: ApprovalTx,
    ) -> Self {
        Self {
            cwd,
            conversation_cwds,
            read_files,
            bash_config,
            approval_tx,
        }
    }

    /// Record that a file was read in a conversation.
    fn record_read(&self, conversation_id: u64, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.read_files
            .lock()
            .expect("read_files lock poisoned")
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
            .expect("read_files lock poisoned")
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
        vec![Bash::as_tool(), Read::as_tool(), Edit::as_tool()]
    }

    fn system_prompt(&self) -> Option<String> {
        let mut prompt = environment_block();
        if let Some(policy) = self.bash_config.prompt_block() {
            prompt.push_str(&policy);
        }
        Some(prompt)
    }

    fn on_before_run(
        &self,
        _agent: &str,
        conversation_id: u64,
        _history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        let cwd = self.effective_cwd(Some(conversation_id));
        vec![
            HistoryEntry::user(format!(
                "<environment>\nworking_directory: {}\n</environment>",
                cwd.display()
            ))
            .auto_injected(),
        ]
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

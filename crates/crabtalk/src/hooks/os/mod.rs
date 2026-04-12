//! OS tools — bash, read, edit — as a Hook implementation.

mod bash;
mod edit;
mod read;

use runtime::{ConversationCwds, Hook};
use std::{fmt::Write, path::PathBuf};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool, model::HistoryEntry};

use bash::Bash;
use edit::Edit;
use read::Read;

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
}

impl OsHook {
    pub fn new(cwd: PathBuf, conversation_cwds: ConversationCwds) -> Self {
        Self {
            cwd,
            conversation_cwds,
        }
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
        Some(environment_block())
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

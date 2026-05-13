//! OS tools — bash, read, edit — as a Hook implementation.
//!
//! Designed for client-side use: the daemon never executes these. A
//! crabtalk client (TUI, custom host) builds one `OsHook` per session
//! against the user's actual cwd, and answers daemon-forwarded
//! `ToolCallForward` events through it.

use bash::Bash;
use edit::Edit;
use parking_lot::Mutex;
use read::Read;
use runtime::Hook;
use std::{
    collections::HashSet,
    fmt::Write,
    path::{Path, PathBuf},
};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool};

mod bash;
mod edit;
mod read;

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
/// Scoped to a single session — the cwd is fixed at construction and the
/// read-file invariant ("must read before edit") is enforced within the
/// instance.
pub struct OsHook {
    cwd: PathBuf,
    /// Files read by this session — edit requires a prior read.
    read_files: Mutex<HashSet<PathBuf>>,
}

impl OsHook {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            read_files: Mutex::new(HashSet::new()),
        }
    }

    /// Record that a file was read.
    fn record_read(&self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        self.read_files.lock().insert(path);
    }

    /// Check whether a file was read.
    fn was_read(&self, path: &Path) -> bool {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.read_files.lock().contains(&path)
    }

    fn effective_cwd(&self) -> &Path {
        &self.cwd
    }

    /// Execute an OS tool by name. The canonical entry point for SDK
    /// consumers that want to run these tools locally (e.g. a TUI
    /// answering a daemon-forwarded `ToolCallForward`). Pure args in,
    /// result out — no `ToolDispatch`, no agent identity, no runtime
    /// trait machinery.
    pub async fn execute(&self, name: &str, args: &str) -> Result<String, String> {
        match name {
            "bash" => self.handle_bash(args).await,
            "read" => self.handle_read(args).await,
            "edit" => self.handle_edit(args).await,
            _ => Err(format!("tool not registered: {name}")),
        }
    }
}

/// The OS tool set's static schemas — bash, read, edit. Hosts that
/// advertise these tools (daemon-side `ClientToolHook` for forwarding,
/// client-side local execution) call this to avoid re-deriving them.
pub fn schemas() -> Vec<wcore::model::Tool> {
    vec![Bash::as_tool(), Read::as_tool(), Edit::as_tool()]
}

/// Names of the OS tool set. Used by `ClientToolHook` to recognise
/// dispatches that should be forwarded to the client.
pub fn names() -> Vec<String> {
    schemas().into_iter().map(|t| t.function.name).collect()
}

/// Walk up from `cwd` collecting `Crab.md` files, plus the global one in
/// the user's config dir. Returned text is the concatenation in
/// deepest-first order (so project-local rules layer over global ones).
///
/// Clients call this each turn and prepend the result, wrapped in
/// `<instructions>…</instructions>`, to the user message they send to
/// the daemon. The daemon does not read the user's filesystem.
pub fn discover_instructions(cwd: &Path) -> Option<String> {
    let config_dir = &*wcore::paths::CONFIG_DIR;
    let mut layers = Vec::new();

    let global = config_dir.join("Crab.md");
    if let Ok(content) = std::fs::read_to_string(&global) {
        layers.push(content);
    }

    let mut found = Vec::new();
    let mut dir = cwd;
    loop {
        let candidate = dir.join("Crab.md");
        if candidate.is_file()
            && !candidate.starts_with(config_dir)
            && let Ok(content) = std::fs::read_to_string(&candidate)
        {
            found.push(content);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    found.reverse();
    layers.extend(found);

    if layers.is_empty() {
        return None;
    }
    Some(layers.join("\n\n"))
}

impl Hook for OsHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        schemas()
    }

    fn system_prompt(&self) -> Option<String> {
        Some(environment_block())
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if !matches!(name, "bash" | "read" | "edit") {
            return None;
        }
        Some(Box::pin(
            async move { self.execute(name, &call.args).await },
        ))
    }
}

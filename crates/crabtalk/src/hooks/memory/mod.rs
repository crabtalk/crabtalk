//! Memory hook — thin facade over `crabtalk-memory`. Per-tool files
//! (`recall.rs`, `remember.rs`, `forget.rs`) own the corresponding
//! `Memory` methods and `MemoryHook` dispatch handlers. See RFC 0150
//! for the design.

use anyhow::Result;
use forget::Forget;
use memory::Memory as Store;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use recall::Recall;
use remember::Remember;
use runtime::Hook;
use std::{path::PathBuf, sync::Arc};
use wcore::{
    MemoryConfig, ToolDispatch, ToolFuture,
    agent::AsTool,
    model::{HistoryEntry, Tool},
};

mod forget;
mod recall;
mod remember;

/// Shared handle to the underlying memory store. Cloneable because the
/// runtime needs a reference of its own for writing archives during
/// compaction and reading them back on session resume.
pub type SharedStore = Arc<RwLock<Store>>;

pub const DEFAULT_SOUL: &str = include_str!("../../../prompts/crab.md");

/// Behavioural guidance for the agent — when/how to use the memory
/// tools. Tool *signatures* come from each struct's `///` doc comment
/// via schemars; this prompt covers everything that doesn't fit in a
/// per-arg description.
const MEMORY_PROMPT: &str = include_str!("../../../prompts/memory.md");

pub struct Memory {
    pub(super) inner: SharedStore,
    pub(super) recall_limit: usize,
}

impl Memory {
    /// Open (or create) the memory db at `db_path`.
    pub fn open(config: MemoryConfig, db_path: PathBuf) -> Result<Self> {
        let store = Store::open(&db_path)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(store)),
            recall_limit: config.recall_limit,
        })
    }

    /// Clone the underlying store handle. Used to hand the same memory
    /// to the runtime for archive writes and resume-time reads.
    pub fn shared(&self) -> SharedStore {
        self.inner.clone()
    }

    pub(super) fn store_read(&self) -> RwLockReadGuard<'_, Store> {
        self.inner.read()
    }

    pub(super) fn store_write(&self) -> RwLockWriteGuard<'_, Store> {
        self.inner.write()
    }
}

pub struct MemoryHook {
    pub(super) memory: Arc<Memory>,
}

impl MemoryHook {
    pub fn new(memory: Arc<Memory>) -> Self {
        Self { memory }
    }
}

impl Hook for MemoryHook {
    fn schema(&self) -> Vec<Tool> {
        vec![Recall::as_tool(), Remember::as_tool(), Forget::as_tool()]
    }

    fn system_prompt(&self) -> Option<String> {
        Some(format!("\n\n{MEMORY_PROMPT}"))
    }

    fn on_before_run(
        &self,
        _agent: &str,
        _conversation_id: u64,
        history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        self.memory.before_run(history)
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        match name {
            "recall" => Some(Box::pin(self.handle_recall(call))),
            "remember" => Some(Box::pin(self.handle_remember(call))),
            "forget" => Some(Box::pin(self.handle_forget(call))),
            _ => None,
        }
    }
}

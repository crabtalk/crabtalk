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
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use wcore::{AgentConfig, MemoryConfig, ToolDispatch, ToolFuture, agent::AsTool, model::Tool};

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
}

impl Memory {
    /// Open (or create) the memory db at `db_path`.
    pub fn open(db_path: PathBuf) -> Result<Self> {
        let store = Store::open(&db_path)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(store)),
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
    /// Per-agent recall limit cache, populated from `on_register_agent`.
    /// Lives on the hook instead of being read from storage on every
    /// `before_run` so the sync hook callbacks don't need an async
    /// roundtrip.
    configs: RwLock<BTreeMap<String, MemoryConfig>>,
}

impl MemoryHook {
    pub fn new(memory: Arc<Memory>) -> Self {
        Self {
            memory,
            configs: RwLock::new(BTreeMap::new()),
        }
    }

    fn recall_limit(&self, agent: &str) -> usize {
        self.configs
            .read()
            .get(agent)
            .map(|c| c.recall_limit)
            .unwrap_or_else(|| MemoryConfig::default().recall_limit)
    }
}

impl Hook for MemoryHook {
    fn schema(&self) -> Vec<Tool> {
        vec![Recall::as_tool(), Remember::as_tool(), Forget::as_tool()]
    }

    fn system_prompt(&self) -> Option<String> {
        Some(format!("\n\n{MEMORY_PROMPT}"))
    }

    fn on_register_agent(&self, name: &str, config: &AgentConfig) {
        self.configs
            .write()
            .insert(name.to_owned(), config.hooks.memory.clone());
    }

    fn on_unregister_agent(&self, name: &str) {
        self.configs.write().remove(name);
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

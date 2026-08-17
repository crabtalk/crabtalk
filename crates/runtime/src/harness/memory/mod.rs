//! Memory hook — the `recall` / `remember` / `forget` tools over the
//! [`Memory`](store::interface::Memory) interface. Per-tool files own
//! the corresponding handlers. See RFC 0150 for the design.
//!
//! The hook holds a store handle, not a store: entries live in the
//! backend and are read by name for the one call that needs them. There
//! is no resident index here — ranking is the backend's.

use crate::Harness;
use crate::{ToolDispatch, ToolFuture, agent::AsTool};
use crabllm_core::Tool;
use forget::Forget;
use parking_lot::RwLock;
use recall::Recall;
use remember::Remember;
use std::{collections::BTreeMap, sync::Arc};
use store::{AgentConfig, AgentId, MemoryConfig, interface::Memory};

mod forget;
mod recall;
mod remember;

/// Behavioural guidance for the agent — when/how to use the memory
/// tools. Tool *signatures* come from each struct's `///` doc comment
/// via schemars; this prompt covers everything that doesn't fit in a
/// per-arg description.
const MEMORY_USAGE: &str = include_str!("../../../prompts/memory.md");

pub struct MemoryHook<M> {
    pub(super) memory: Arc<M>,
    /// Per-agent recall limit, refreshed on every resolve. Kept here so
    /// the sync hook callbacks and `before_run` never need an async
    /// roundtrip to read one number.
    configs: RwLock<BTreeMap<AgentId, MemoryConfig>>,
}

impl<M: Memory> MemoryHook<M> {
    pub fn new(memory: Arc<M>) -> Self {
        Self {
            memory,
            configs: RwLock::new(BTreeMap::new()),
        }
    }

    fn recall_limit(&self, agent: &AgentId) -> usize {
        self.configs
            .read()
            .get(agent)
            .map(|c| c.recall_limit)
            .unwrap_or_else(|| MemoryConfig::default().recall_limit)
    }
}

impl<M: Memory + 'static> Harness for MemoryHook<M> {
    fn schema(&self) -> Vec<Tool> {
        vec![Recall::as_tool(), Remember::as_tool(), Forget::as_tool()]
    }

    fn usage(&self) -> Option<String> {
        Some(format!("\n\n{MEMORY_USAGE}"))
    }

    fn on_resolve_agent(&self, id: &AgentId, config: &AgentConfig) {
        self.configs
            .write()
            .insert(*id, config.hooks.memory.clone());
    }

    fn on_forget_agent(&self, id: &AgentId) {
        self.configs.write().remove(id);
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

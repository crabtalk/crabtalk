//! Memory tools — recall, remember, forget, memory — as a Hook implementation.

use super::Memory;
use runtime::Hook;
use serde::Deserialize;
use std::sync::Arc;
use wcore::{
    ToolDispatch, ToolFuture,
    agent::{AsTool, ToolDescription},
    model::HistoryEntry,
    storage::Storage,
};

// ── Schemas ──────────────────────────────────────────────────────

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Recall {
    /// Keyword or phrase to search your memory entries for.
    pub query: String,
    /// Maximum number of results to return. Defaults to 5.
    pub limit: Option<usize>,
}

impl ToolDescription for Recall {
    const DESCRIPTION: &'static str =
        "Search your memory entries by keyword. Returns ranked results.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Remember {
    /// Short name for this memory entry (used as identifier).
    pub name: String,
    /// One-line description — determines search relevance.
    pub description: String,
    /// The content to remember.
    pub content: String,
}

impl ToolDescription for Remember {
    const DESCRIPTION: &'static str = "Save or update a memory entry. Creates a persistent file with the given name, description, and content.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Forget {
    /// Name of the memory entry to delete.
    pub name: String,
}

impl ToolDescription for Forget {
    const DESCRIPTION: &'static str = "Delete a memory entry by name.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemoryTool {
    /// The full content to write to MEMORY.md — your curated overview.
    pub content: String,
}

impl ToolDescription for MemoryTool {
    const DESCRIPTION: &'static str = "Overwrite MEMORY.md — your curated overview injected every session. Read it before overwriting.";
}

// ── Hook ────────────────────────────────────────────────────────

/// Memory subsystem: recall, remember, forget, memory.
///
/// Owns the Memory index and provides auto-recall in on_before_run.
pub struct MemoryHook<S: Storage> {
    memory: Arc<Memory<S>>,
}

impl<S: Storage> MemoryHook<S> {
    pub fn new(memory: Arc<Memory<S>>) -> Self {
        Self { memory }
    }
}

impl<S: Storage + 'static> Hook for MemoryHook<S> {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![
            Recall::as_tool(),
            Remember::as_tool(),
            Forget::as_tool(),
            MemoryTool::as_tool(),
        ]
    }

    fn system_prompt(&self) -> Option<String> {
        Some(self.memory.build_prompt())
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
            "recall" => Some(Box::pin(async move {
                let input: Recall = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(self.memory.recall(&input.query, input.limit.unwrap_or(5)))
            })),
            "remember" => Some(Box::pin(async move {
                let input: Remember = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(self
                    .memory
                    .remember(input.name, input.description, input.content))
            })),
            "forget" => Some(Box::pin(async move {
                let input: Forget = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(self.memory.forget(&input.name))
            })),
            "memory" => Some(Box::pin(async move {
                let input: MemoryTool = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(self.memory.write_index(&input.content))
            })),
            _ => None,
        }
    }
}

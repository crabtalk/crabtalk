//! Memory tools — recall, remember, forget, memory — as a Hook implementation.

use super::Memory;
use runtime::Hook;
use serde::Deserialize;
use std::sync::Arc;
use wcore::{
    ToolDispatch, ToolFuture,
    agent::{AsTool, ToolDescription},
    model::HistoryEntry,
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
    /// The content to remember — markdown.
    pub content: String,
    /// Optional alternative search terms / related note names.
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl ToolDescription for Remember {
    const DESCRIPTION: &'static str =
        "Save or update a memory entry. Aliases are searchable alternative terms.";
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

pub struct MemoryHook {
    memory: Arc<Memory>,
}

impl MemoryHook {
    pub fn new(memory: Arc<Memory>) -> Self {
        Self { memory }
    }
}

impl Hook for MemoryHook {
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
                    .remember(input.name, input.content, input.aliases))
            })),
            "forget" => Some(Box::pin(async move {
                let input: Forget = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(self.memory.forget(&input.name))
            })),
            "memory" => Some(Box::pin(async move {
                let input: MemoryTool = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(self.memory.write_prompt(&input.content))
            })),
            _ => None,
        }
    }
}

//! Memory tool handler factories — recall, remember, forget, memory.

use super::Memory;
use serde::Deserialize;
use std::sync::Arc;
use wcore::{
    ToolDispatch, ToolEntry,
    agent::{AsTool, ToolDescription},
    repos::Storage,
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

// ── Handlers ─────────────────────────────────────────────────────

pub fn handlers<S: Storage + 'static>(memory: Arc<Memory<S>>) -> Vec<ToolEntry> {
    // Recall gets system_prompt (memory index) and before_run (auto-recall).
    let m = memory.clone();
    let m2 = memory.clone();
    let recall = ToolEntry {
        schema: Recall::as_tool(),
        system_prompt: Some(m.build_prompt()),
        before_run: Some(Arc::new(move |history| m2.before_run(history))),
        handler: Arc::new(move |call: ToolDispatch| {
            let mem = m.clone();
            Box::pin(async move {
                let input: Recall = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(mem.recall(&input.query, input.limit.unwrap_or(5)))
            })
        }),
    };

    let m = memory.clone();
    let remember = ToolEntry {
        schema: Remember::as_tool(),
        system_prompt: None,
        before_run: None,
        handler: Arc::new(move |call: ToolDispatch| {
            let mem = m.clone();
            Box::pin(async move {
                let input: Remember = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(mem.remember(input.name, input.description, input.content))
            })
        }),
    };

    let m = memory.clone();
    let forget = ToolEntry {
        schema: Forget::as_tool(),
        system_prompt: None,
        before_run: None,
        handler: Arc::new(move |call: ToolDispatch| {
            let mem = m.clone();
            Box::pin(async move {
                let input: Forget = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(mem.forget(&input.name))
            })
        }),
    };

    let m = memory;
    let memory_tool = ToolEntry {
        schema: MemoryTool::as_tool(),
        system_prompt: None,
        before_run: None,
        handler: Arc::new(move |call: ToolDispatch| {
            let mem = m.clone();
            Box::pin(async move {
                let input: MemoryTool = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                Ok(mem.write_index(&input.content))
            })
        }),
    };

    vec![recall, remember, forget, memory_tool]
}

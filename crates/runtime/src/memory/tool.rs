//! Tool schemas and dispatch for built-in memory tools.

use crate::{Env, host::Host};
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

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

pub fn tools() -> Vec<Tool> {
    vec![
        Recall::as_tool(),
        Remember::as_tool(),
        Forget::as_tool(),
        MemoryTool::as_tool(),
    ]
}

impl<H: Host> Env<H> {
    pub async fn dispatch_recall(&self, args: &str) -> Result<String, String> {
        let input: Recall = serde_json::from_str(args)
            .map_err(|e| format!("invalid arguments: {e}"))?;
        let mem = self.memory.as_ref().ok_or("memory not available")?;
        Ok(mem.recall(&input.query, input.limit.unwrap_or(5)))
    }

    pub async fn dispatch_remember(&self, args: &str) -> Result<String, String> {
        let input: Remember = serde_json::from_str(args)
            .map_err(|e| format!("invalid arguments: {e}"))?;
        let mem = self.memory.as_ref().ok_or("memory not available")?;
        Ok(mem.remember(input.name, input.description, input.content))
    }

    pub async fn dispatch_forget(&self, args: &str) -> Result<String, String> {
        let input: Forget = serde_json::from_str(args)
            .map_err(|e| format!("invalid arguments: {e}"))?;
        let mem = self.memory.as_ref().ok_or("memory not available")?;
        Ok(mem.forget(&input.name))
    }

    pub async fn dispatch_memory(&self, args: &str) -> Result<String, String> {
        let input: MemoryTool = serde_json::from_str(args)
            .map_err(|e| format!("invalid arguments: {e}"))?;
        let mem = self.memory.as_ref().ok_or("memory not available")?;
        Ok(mem.write_index(&input.content))
    }
}

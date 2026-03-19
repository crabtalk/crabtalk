//! Tool schemas and dispatch for built-in memory tools.
//!
//! Five tools: recall, remember, forget, memory (MEMORY.md), soul (Crab.md).

use crate::hook::DaemonHook;
use serde::Deserialize;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Recall {
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
pub(crate) struct Remember {
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
pub(crate) struct Forget {
    /// Name of the memory entry to delete.
    pub name: String,
}

impl ToolDescription for Forget {
    const DESCRIPTION: &'static str = "Delete a memory entry by name.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Memory {
    /// The full content to write to MEMORY.md — your curated overview.
    pub content: String,
}

impl ToolDescription for Memory {
    const DESCRIPTION: &'static str = "Overwrite MEMORY.md — your curated overview injected every session. Read it before overwriting.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Soul {
    /// The full content to write to Crab.md — your identity and personality.
    pub content: String,
}

impl ToolDescription for Soul {
    const DESCRIPTION: &'static str = "Overwrite Crab.md — your identity and personality. Only edit when the user explicitly shapes who you are.";
}

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Recall::as_tool(),
        Remember::as_tool(),
        Forget::as_tool(),
        Memory::as_tool(),
        Soul::as_tool(),
    ]
}

impl DaemonHook {
    pub(crate) async fn dispatch_recall(&self, args: &str) -> String {
        let input: Recall = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match self.memory {
            Some(ref mem) => mem.recall(&input.query, input.limit.unwrap_or(5)),
            None => "memory not available".to_owned(),
        }
    }

    pub(crate) async fn dispatch_remember(&self, args: &str) -> String {
        let input: Remember = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match self.memory {
            Some(ref mem) => mem.remember(input.name, input.description, input.content),
            None => "memory not available".to_owned(),
        }
    }

    pub(crate) async fn dispatch_forget(&self, args: &str) -> String {
        let input: Forget = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match self.memory {
            Some(ref mem) => mem.forget(&input.name),
            None => "memory not available".to_owned(),
        }
    }

    pub(crate) async fn dispatch_memory(&self, args: &str) -> String {
        let input: Memory = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match self.memory {
            Some(ref mem) => mem.write_index(&input.content),
            None => "memory not available".to_owned(),
        }
    }

    pub(crate) async fn dispatch_soul(&self, args: &str) -> String {
        let input: Soul = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        match self.memory {
            Some(ref mem) => mem.write_soul(&input.content),
            None => "memory not available".to_owned(),
        }
    }
}

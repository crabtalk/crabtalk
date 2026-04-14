//! `memory` — overwrite the reserved `global` Prompt entry (what used
//! to be `MEMORY.md`). Also owns `build_prompt`, the read side that
//! assembles the system-prompt block from the same entry.
//!
//! `#[schemars(rename = "Memory")]` pins the tool name to `memory` —
//! without it, the derived schema name would be `prompt`, which doesn't
//! match the dispatch match arm or the prompt the agent reads.

use super::{GLOBAL_PROMPT_NAME, MEMORY_PROMPT, Memory, MemoryHook};
use memory::{EntryKind, Op};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Overwrite MEMORY.md — your curated overview injected every session. Read it before overwriting.
#[derive(Deserialize, JsonSchema)]
#[schemars(rename = "Memory")]
pub struct Prompt {
    /// The full content to write to MEMORY.md — your curated overview.
    pub content: String,
}

impl Memory {
    /// Upsert the reserved `global` Prompt entry (what `MEMORY.md` used
    /// to be).
    pub fn write_prompt(&self, content: &str) -> String {
        let mut store = self.store_write();
        let exists = store.get(GLOBAL_PROMPT_NAME).is_some();
        let op = if exists {
            Op::Update {
                name: GLOBAL_PROMPT_NAME.to_owned(),
                content: content.to_owned(),
                aliases: vec![],
            }
        } else {
            Op::Add {
                name: GLOBAL_PROMPT_NAME.to_owned(),
                content: content.to_owned(),
                aliases: vec![],
                kind: EntryKind::Prompt,
            }
        };
        match store.apply(op) {
            Ok(_) => "MEMORY.md updated".to_owned(),
            Err(e) => format!("failed to write MEMORY.md: {e}"),
        }
    }

    /// System-prompt block: the `global` Prompt content wrapped in
    /// `<memory>` tags, plus the memory tool instructions.
    pub fn build_prompt(&self) -> String {
        let store = self.store_read();
        match store.get(GLOBAL_PROMPT_NAME) {
            Some(e) if !e.content.trim().is_empty() => {
                format!("\n\n<memory>\n{}\n</memory>\n\n{MEMORY_PROMPT}", e.content)
            }
            _ => format!("\n\n{MEMORY_PROMPT}"),
        }
    }
}

impl MemoryHook {
    pub(super) async fn handle_memory(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Prompt =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        Ok(self.memory.write_prompt(&input.content))
    }
}

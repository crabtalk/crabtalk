//! `recall` — ranked search over memory entries.

use crate::ToolDispatch;
use crate::harness::memory::MemoryHook;
use schemars::JsonSchema;
use serde::Deserialize;
use store::interface::Memory;

/// Search your memory entries by keyword. Returns ranked results.
#[derive(Deserialize, JsonSchema)]
pub struct Recall {
    /// Keyword or phrase to search your memory entries for.
    pub query: String,
    /// Maximum number of results to return. Defaults to 5.
    pub limit: Option<usize>,
}

impl<M: Memory> MemoryHook<M> {
    /// Search, then read back only the entries that ranked. The index
    /// returns names; bodies are fetched for the handful kept, so a
    /// broad query never drags the whole store into the reply.
    pub async fn recall(&self, query: &str, limit: usize) -> String {
        let names = match self.memory.search_memory(query, limit).await {
            Ok(names) => names,
            Err(e) => return format!("recall failed: {e}"),
        };
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if let Ok(Some(entry)) = self.memory.memory(&name).await {
                out.push(format!("## {}\n{}", entry.name, entry.content));
            }
        }
        if out.is_empty() {
            return "no memories found".to_owned();
        }
        out.join("\n---\n")
    }

    pub(super) async fn handle_recall(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Recall =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        let limit = input
            .limit
            .unwrap_or_else(|| self.recall_limit(&call.agent));
        Ok(self.recall(&input.query, limit).await)
    }
}

//! `recall` — BM25 search over memory entries.

use super::{Memory, MemoryHook};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Search your memory entries by keyword. Returns ranked results.
#[derive(Deserialize, JsonSchema)]
pub struct Recall {
    /// Keyword or phrase to search your memory entries for.
    pub query: String,
    /// Maximum number of results to return. Defaults to 5.
    pub limit: Option<usize>,
}

impl Memory {
    pub fn recall(&self, query: &str, limit: usize) -> String {
        let store = self.store_read();
        let hits = store.search(query, limit);
        if hits.is_empty() {
            return "no memories found".to_owned();
        }
        hits.iter()
            .map(|h| format!("## {}\n{}", h.entry.name, h.entry.content))
            .collect::<Vec<_>>()
            .join("\n---\n")
    }
}

impl MemoryHook {
    pub(super) async fn handle_recall(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Recall =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        let limit = input
            .limit
            .unwrap_or_else(|| self.recall_limit(&call.agent));
        Ok(self.memory.recall(&input.query, limit))
    }
}

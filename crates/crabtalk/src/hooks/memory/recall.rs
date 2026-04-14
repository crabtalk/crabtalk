//! `recall` — BM25 search over memory entries. Also owns the
//! before-run auto-recall hook, which is just a recall driven by the
//! last user message.

use super::{Memory, MemoryHook};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::{
    ToolDispatch,
    model::{HistoryEntry, Role},
};

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

    /// Auto-recall: BM25-search the last user message, inject any hits
    /// as a synthetic user turn.
    pub fn before_run(&self, history: &[HistoryEntry]) -> Vec<HistoryEntry> {
        let last_user = history
            .iter()
            .rev()
            .find(|e| *e.role() == Role::User && !e.text().is_empty());

        let Some(entry) = last_user else {
            return Vec::new();
        };

        let query: String = entry
            .text()
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");

        if query.is_empty() {
            return Vec::new();
        }

        let result = self.recall(&query, self.recall_limit);
        if result == "no memories found" {
            return Vec::new();
        }
        vec![HistoryEntry::user(format!("<recall>\n{result}\n</recall>")).auto_injected()]
    }
}

impl MemoryHook {
    pub(super) async fn handle_recall(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Recall =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        Ok(self.memory.recall(&input.query, input.limit.unwrap_or(5)))
    }
}

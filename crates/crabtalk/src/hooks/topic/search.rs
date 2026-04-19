//! `search_topics` — BM25 search restricted to `EntryKind::Topic`.

use super::TopicHook;
use crabllm_core::Provider;
use memory::EntryKind;
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Search your existing topics by keyword. Returns ranked
/// `(title, description)` pairs.
#[derive(Deserialize, JsonSchema)]
pub struct SearchTopics {
    /// Keyword or phrase to match against topic titles/descriptions.
    pub query: String,
    /// Maximum number of results to return. Defaults to 5.
    pub limit: Option<usize>,
}

impl<P: Provider + 'static> TopicHook<P> {
    pub(super) async fn handle_search_topics(&self, call: ToolDispatch) -> Result<String, String> {
        let input: SearchTopics =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        let limit = input.limit.unwrap_or(5);
        let store = self.memory.read();
        let hits = store.search_kind(&input.query, limit, EntryKind::Topic);
        if hits.is_empty() {
            return Ok("no topics found".to_owned());
        }
        Ok(hits
            .iter()
            .map(|h| format!("## {}\n{}", h.entry.name, h.entry.content))
            .collect::<Vec<_>>()
            .join("\n---\n"))
    }
}

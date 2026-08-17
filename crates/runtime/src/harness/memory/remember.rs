//! `remember` — upsert a memory entry.

use crate::ToolDispatch;
use crate::harness::memory::MemoryHook;
use schemars::JsonSchema;
use serde::Deserialize;
use store::{MemoryEntry, interface::Memory};

/// Save or update a memory entry. Aliases are searchable alternative terms.
#[derive(Deserialize, JsonSchema)]
pub struct Remember {
    /// Short name for this memory entry (used as identifier).
    pub name: String,
    /// The content to remember — markdown.
    pub content: String,
    /// Optional alternative search terms / related note names.
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl<M: Memory> MemoryHook<M> {
    /// Upsert. `created_at` is preserved when the entry already exists,
    /// so re-remembering does not reset when it was first learned.
    pub async fn remember(&self, name: String, content: String, aliases: Vec<String>) -> String {
        let created_at = match self.memory.memory(&name).await {
            Ok(Some(existing)) => existing.created_at,
            _ => chrono::Utc::now().to_rfc3339(),
        };
        let entry = MemoryEntry {
            name: name.clone(),
            kind: "note".to_owned(),
            content,
            aliases,
            created_at,
        };
        match self.memory.put_memory(&entry).await {
            Ok(()) => format!("remembered: {name}"),
            Err(e) => format!("failed to save entry: {e}"),
        }
    }

    pub(super) async fn handle_remember(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Remember =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        Ok(self
            .remember(input.name, input.content, input.aliases)
            .await)
    }
}

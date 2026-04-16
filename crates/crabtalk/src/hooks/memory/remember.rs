//! `remember` — upsert a memory entry as an `EntryKind::Note`.

use super::{Memory, MemoryHook};
use memory::{EntryKind, Op};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

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

impl Memory {
    pub fn remember(&self, name: String, content: String, aliases: Vec<String>) -> String {
        let mut store = self.store_write();
        let exists = store.get(&name).is_some();
        let op = if exists {
            Op::Update {
                name: name.clone(),
                content,
                aliases,
            }
        } else {
            Op::Add {
                name: name.clone(),
                content,
                aliases,
                kind: EntryKind::Note,
            }
        };
        match store.apply(op) {
            Ok(_) => format!("remembered: {name}"),
            Err(e) => format!("failed to save entry: {e}"),
        }
    }
}

impl MemoryHook {
    pub(super) async fn handle_remember(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Remember =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        Ok(self
            .memory
            .remember(input.name, input.content, input.aliases))
    }
}

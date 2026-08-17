//! `forget` — delete a memory entry by name.

use crate::ToolDispatch;
use crate::harness::memory::MemoryHook;
use schemars::JsonSchema;
use serde::Deserialize;
use store::interface::Memory;

/// Delete a memory entry by name.
#[derive(Deserialize, JsonSchema)]
pub struct Forget {
    /// Name of the memory entry to delete.
    pub name: String,
}

impl<M: Memory> MemoryHook<M> {
    pub async fn forget(&self, name: &str) -> String {
        match self.memory.remove_memory(name).await {
            Ok(true) => format!("forgot: {name}"),
            Ok(false) => format!("no entry named: {name}"),
            Err(e) => format!("failed to forget {name}: {e}"),
        }
    }

    pub(super) async fn handle_forget(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Forget =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        Ok(self.forget(&input.name).await)
    }
}

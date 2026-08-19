//! `forget` — delete a memory entry by name.

use super::MemoryHarness;
use runtime::ToolDispatch;
use schemars::JsonSchema;
use serde::Deserialize;
use store::interface::Memory;

/// Delete a memory entry by name.
#[derive(Deserialize, JsonSchema)]
pub struct Forget {
    /// Name of the memory entry to delete.
    pub name: String,
}

impl<M: Memory> MemoryHarness<M> {
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

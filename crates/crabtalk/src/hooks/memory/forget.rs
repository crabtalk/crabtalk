//! `forget` — delete a memory entry by name.

use super::{Memory, MemoryHook};
use memory::Op;
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::ToolDispatch;

/// Delete a memory entry by name.
#[derive(Deserialize, JsonSchema)]
pub struct Forget {
    /// Name of the memory entry to delete.
    pub name: String,
}

impl Memory {
    pub fn forget(&self, name: &str) -> String {
        let mut store = self.store_write();
        match store.apply(Op::Remove {
            name: name.to_owned(),
        }) {
            Ok(_) => format!("forgot: {name}"),
            Err(_) => format!("no entry named: {name}"),
        }
    }
}

impl MemoryHook {
    pub(super) async fn handle_forget(&self, call: ToolDispatch) -> Result<String, String> {
        let input: Forget =
            serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
        Ok(self.memory.forget(&input.name))
    }
}

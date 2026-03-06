//! Memory tool schema constructors.
//!
//! Returns [`Tool`] schema definitions for `remember` and `recall`.
//! No handlers — dispatch is handled statically by the daemon event loop.

use crate::model::Tool;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
pub struct RememberInput {
    /// Memory key
    pub key: String,
    /// Value to remember
    pub value: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RecallInput {
    /// Search query for relevant memories
    pub query: String,
    /// Maximum number of results (default: 10)
    pub limit: Option<u32>,
}

/// Build the `remember` tool schema.
pub fn remember_schema() -> Tool {
    Tool {
        name: "remember".into(),
        description: "Store a key-value pair in memory.".into(),
        parameters: schemars::schema_for!(RememberInput),
        strict: false,
    }
}

/// Build the `recall` tool schema.
pub fn recall_schema() -> Tool {
    Tool {
        name: "recall".into(),
        description: "Search memory for entries relevant to a query.".into(),
        parameters: schemars::schema_for!(RecallInput),
        strict: false,
    }
}

//! Bash tool — schema definition.

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) mod config;
mod handler;

/// Run a shell command.
#[derive(Deserialize, JsonSchema)]
pub struct Bash {
    /// Shell command to run (e.g. `"ls -la"`, `"cat foo.txt | grep bar"`).
    pub command: String,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

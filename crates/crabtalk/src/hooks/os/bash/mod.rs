//! Bash tool — schema definition.

pub(super) mod config;
mod handler;

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Run a shell command.
#[derive(Deserialize, JsonSchema)]
pub struct Bash {
    /// Shell command to run (e.g. `"ls -la"`, `"cat foo.txt | grep bar"`).
    pub command: String,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

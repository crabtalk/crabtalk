//! Bash tool — schema definition.

mod config;
mod handler;

pub use config::BashConfig;

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use wcore::agent::ToolDescription;

#[derive(Deserialize, JsonSchema)]
pub struct Bash {
    /// Shell command to run (e.g. `"ls -la"`, `"cat foo.txt | grep bar"`).
    pub command: String,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl ToolDescription for Bash {
    const DESCRIPTION: &'static str = "Run a shell command.";
}

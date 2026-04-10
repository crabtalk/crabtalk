//! Tool schema for the MCP meta-tool.

use schemars::JsonSchema;
use serde::Deserialize;
use wcore::agent::{AsTool, ToolDescription};

#[derive(Deserialize, JsonSchema)]
pub struct Mcp {
    /// Tool name to call. If no exact match, returns fuzzy matches.
    /// Leave empty to list all available MCP tools.
    pub name: String,
    /// JSON-encoded arguments string (only used when calling a tool).
    #[serde(default)]
    pub args: Option<String>,
}

impl ToolDescription for Mcp {
    const DESCRIPTION: &'static str =
        "Call an MCP tool by name, or list available tools if no exact match.";
}

pub fn tools() -> Vec<wcore::model::Tool> {
    vec![Mcp::as_tool()]
}

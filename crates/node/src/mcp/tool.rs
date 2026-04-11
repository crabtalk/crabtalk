//! MCP tool schema and handler factory.

use super::McpHandler;
use runtime::AgentScope;
use schemars::JsonSchema;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use wcore::{ToolDispatch, ToolHandler, agent::ToolDescription};

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

/// Build a handler that dispatches MCP tool calls through the McpHandler.
pub fn handler(
    mcp: Arc<McpHandler>,
    scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
) -> ToolHandler {
    Arc::new(move |call: ToolDispatch| {
        let mcp = mcp.clone();
        let scopes = scopes.clone();
        Box::pin(async move {
            let allowed_mcps: Vec<String> = scopes
                .read()
                .expect("scopes lock poisoned")
                .get(&call.agent)
                .filter(|s| !s.mcps.is_empty())
                .map(|s| s.mcps.clone())
                .unwrap_or_default();
            super::dispatch::dispatch_mcp(&mcp, &call.args, &allowed_mcps).await
        })
    })
}

//! MCP tool — as a Hook implementation.

use crate::daemon::hook::AgentScope;
use mcp::{McpHandler, dispatch::dispatch_mcp};
use parking_lot::RwLock;
use runtime::Hook;
use schemars::JsonSchema;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool};

/// Call an MCP tool by name, or list available tools if no exact match.
#[derive(Deserialize, JsonSchema)]
pub struct Mcp {
    /// Tool name to call. If no exact match, returns fuzzy matches.
    /// Leave empty to list all available MCP tools.
    pub name: String,
    /// JSON-encoded arguments string (only used when calling a tool).
    #[serde(default)]
    pub args: Option<String>,
}

/// MCP subsystem: routes tool calls to MCP servers.
pub struct McpHook {
    mcp: Arc<McpHandler>,
    scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
}

impl McpHook {
    pub fn new(mcp: Arc<McpHandler>, scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>) -> Self {
        Self { mcp, scopes }
    }
}

impl Hook for McpHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![Mcp::as_tool()]
    }

    fn scoped_tools(&self, config: &wcore::AgentConfig) -> (Vec<String>, Option<String>) {
        if config.mcps.is_empty() {
            return (vec![], None);
        }
        let tools = self
            .schema()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        let names: Vec<&str> = config.mcps.iter().map(|s| s.as_str()).collect();
        let line = format!("mcp servers: {}", names.join(", "));
        (tools, Some(line))
    }

    fn system_prompt(&self) -> Option<String> {
        let names: Vec<String> = self.mcp.cached_list().into_iter().map(|(n, _)| n).collect();
        if names.is_empty() {
            return None;
        }
        Some(format!(
            "\n\n<resources>\nMCP servers: {}. Use the mcp tool to list or call tools.\n</resources>",
            names.join(", ")
        ))
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if name != "mcp" {
            return None;
        }
        Some(Box::pin(async move {
            let allowed_mcps: Vec<String> = self
                .scopes
                .read()
                .get(&call.agent)
                .filter(|s| !s.mcps.is_empty())
                .map(|s| s.mcps.clone())
                .unwrap_or_default();
            dispatch_mcp(&self.mcp, &call.args, &allowed_mcps).await
        }))
    }
}

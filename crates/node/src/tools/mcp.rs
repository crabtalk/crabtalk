//! MCP tool handler factory.

use crate::mcp::McpHandler;
use runtime::AgentScope;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use wcore::{ToolDispatch, ToolHandler};

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
            crate::mcp::dispatch::dispatch_mcp(&mcp, &call.args, &allowed_mcps).await
        })
    })
}

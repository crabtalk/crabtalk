//! Tool dispatch and schema registration for the MCP tool.

use crate::{RuntimeHook, bridge::RuntimeBridge};
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::agent::ToolDescription;

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

impl<B: RuntimeBridge> RuntimeHook<B> {
    pub async fn dispatch_mcp(&self, args: &str, agent: &str) -> String {
        let input: Mcp = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };

        let bridge = self.mcp.bridge().await;

        // Resolve allowed tools from agent's MCP scope.
        let allowed_tools: Option<Vec<String>> = if let Some(scope) = self.scopes.get(agent)
            && !scope.mcps.is_empty()
        {
            let servers = bridge.list_servers().await;
            Some(
                servers
                    .into_iter()
                    .filter(|(name, _)| scope.mcps.iter().any(|m| m == name.as_str()))
                    .flat_map(|(_, tools)| tools)
                    .collect(),
            )
        } else {
            None
        };

        // Try exact call first.
        if !input.name.is_empty() {
            // Enforce scope.
            if let Some(ref allowed) = allowed_tools
                && !allowed.iter().any(|t| t.as_str() == input.name)
            {
                return format!("tool not available: {}", input.name);
            }

            let tools = bridge.tools().await;
            if tools.iter().any(|t| t.name == input.name) {
                let tool_args = input.args.unwrap_or_default();
                return bridge.call(&input.name, &tool_args).await;
            }
        }

        // No exact match — fuzzy search / list all.
        let query = input.name.to_lowercase();
        let tools = bridge.tools().await;
        let matches: Vec<String> = tools
            .iter()
            .filter(|t| {
                if let Some(ref allowed) = allowed_tools
                    && !allowed.iter().any(|a| a.as_str() == t.name.as_str())
                {
                    return false;
                }
                query.is_empty()
                    || t.name.to_lowercase().contains(&query)
                    || t.description.to_lowercase().contains(&query)
            })
            .map(|t| format!("{}: {}", t.name, t.description))
            .collect();

        if matches.is_empty() {
            "no tools found".to_owned()
        } else {
            matches.join("\n")
        }
    }
}

//! Tool dispatch and schema registration for the MCP tool.

use crate::{Env, host::Host};
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

impl<H: Host> Env<H> {
    pub async fn dispatch_mcp(&self, args: &str, agent: &str) -> Result<String, String> {
        let input: Mcp =
            serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

        let bridge = self.mcp.bridge().await;

        // Resolve allowed tools from agent's MCP scope. Snapshot the scope
        // entry so we don't hold the scopes lock across the bridge await.
        let scoped_mcps: Option<Vec<String>> = self
            .scopes
            .read()
            .expect("scopes lock poisoned")
            .get(agent)
            .filter(|s| !s.mcps.is_empty())
            .map(|s| s.mcps.clone());
        let allowed_tools: Option<Vec<String>> = if let Some(mcps) = scoped_mcps {
            let servers = bridge.list_servers().await;
            Some(
                servers
                    .into_iter()
                    .filter(|(name, _)| mcps.iter().any(|m| m == name.as_str()))
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
                return Err(format!("tool not available: {}", input.name));
            }

            let tools = bridge.tools().await;
            if tools.iter().any(|t| t.function.name == input.name) {
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
                    && !allowed
                        .iter()
                        .any(|a| a.as_str() == t.function.name.as_str())
                {
                    return false;
                }
                query.is_empty()
                    || t.function.name.to_lowercase().contains(&query)
                    || t.function
                        .description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&query))
            })
            .map(|t| {
                format!(
                    "{}: {}",
                    t.function.name,
                    t.function.description.as_deref().unwrap_or(""),
                )
            })
            .collect();

        // Empty discovery is not a failure — the caller asked "what matches?"
        // and got "nothing". Return Ok so the UI doesn't flag it as an error.
        if matches.is_empty() {
            Ok("no tools found".to_owned())
        } else {
            Ok(matches.join("\n"))
        }
    }
}

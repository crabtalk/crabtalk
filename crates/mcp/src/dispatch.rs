//! MCP tool dispatch — the `mcp` meta-tool handler.
//!
//! Called from `McpHook::dispatch` with the calling agent and the
//! parsed args. Owns tool resolution scoped to the agent's declared
//! MCPs, fuzzy matching, and bridge call routing.

use crate::McpHandler;
use serde::Deserialize;

#[derive(Deserialize)]
struct McpArgs {
    name: String,
    #[serde(default)]
    args: Option<String>,
}

/// Dispatch the `mcp` meta-tool.
///
/// `agent` is the calling agent's name; `allowed_mcp_names` are the MCP
/// server names that agent declared. The handler resolves those names
/// to fingerprint-keyed peers and only exposes their tools.
pub async fn dispatch_mcp(
    handler: &McpHandler,
    agent: &str,
    args: &str,
    allowed_mcp_names: &[String],
) -> Result<String, String> {
    let input: McpArgs =
        serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

    let allowed_tools = handler.allowed_tools(agent, allowed_mcp_names);
    let bridge = handler.bridge().await;

    // Try exact call first.
    if !input.name.is_empty() {
        if !allowed_tools.iter().any(|t| t == &input.name) {
            return Err(format!("tool not available: {}", input.name));
        }
        let tool_args = input.args.unwrap_or_default();
        return bridge.call(&input.name, &tool_args).await;
    }

    // No exact match — fuzzy search / list all.
    let query = input.name.to_lowercase();
    let tools = bridge.tools().await;
    let matches: Vec<String> = tools
        .iter()
        .filter(|t| {
            allowed_tools
                .iter()
                .any(|a| a.as_str() == t.function.name.as_str())
        })
        .filter(|t| {
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

    if matches.is_empty() {
        Ok("no tools found".to_owned())
    } else {
        Ok(matches.join("\n"))
    }
}

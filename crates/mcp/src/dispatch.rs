//! MCP tool dispatch — the `mcp` meta-tool handler.
//!
//! Called from `NodeEnv::dispatch_mcp` with the parsed args and the
//! agent's MCP scope. Owns tool resolution, scope enforcement, fuzzy
//! matching, and bridge call routing.

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
/// `allowed_mcps` is the agent's MCP scope — server names the agent may
/// access. Empty means unrestricted.
pub async fn dispatch_mcp(
    handler: &McpHandler,
    args: &str,
    allowed_mcps: &[String],
) -> Result<String, String> {
    let input: McpArgs =
        serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

    let bridge = handler.bridge().await;

    // Resolve which tools this agent may access.
    let allowed_tools: Option<Vec<String>> = if !allowed_mcps.is_empty() {
        let servers = bridge.list_servers().await;
        Some(
            servers
                .into_iter()
                .filter(|(name, _)| allowed_mcps.iter().any(|m| m == name.as_str()))
                .flat_map(|(_, tools)| tools)
                .collect(),
        )
    } else {
        None
    };

    // Try exact call first.
    if !input.name.is_empty() {
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

    if matches.is_empty() {
        Ok("no tools found".to_owned())
    } else {
        Ok(matches.join("\n"))
    }
}

//! MCP tool dispatch — the `mcp` meta-tool handler.
//!
//! Called from `McpHook::dispatch` with the calling agent and the
//! parsed args. Tool routing is keyed by `(peer_id, tool_name)` — two
//! agents may declare different MCPs that both expose a tool with the
//! same name without collision.

use crate::McpHandler;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct McpArgs {
    name: String,
    #[serde(default)]
    args: Option<String>,
}

/// Dispatch the `mcp` meta-tool.
pub async fn dispatch_mcp(
    handler: &McpHandler,
    agent: &str,
    args: &str,
    allowed_mcp_names: &[String],
) -> Result<String, String> {
    let input: McpArgs =
        serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;

    let allowed = handler.allowed(agent, allowed_mcp_names);
    let bridge = handler.bridge().await;

    // Try exact call first. First declarer wins on name collisions.
    if !input.name.is_empty() {
        let Some((peer_id, _)) = allowed.iter().find(|(_, n)| n == &input.name) else {
            return Err(format!("tool not available: {}", input.name));
        };
        let tool_args = input.args.unwrap_or_default();
        return bridge.call(peer_id, &input.name, &tool_args).await;
    }

    // No exact name — fuzzy / list. Pull tool defs only for peers the
    // agent has access to, then filter by query.
    let unique_peers: BTreeSet<String> = allowed.iter().map(|(p, _)| p.clone()).collect();
    let peer_ids: Vec<String> = unique_peers.into_iter().collect();
    let tools = bridge.tools_for(&peer_ids).await;
    let query = input.name.to_lowercase();
    let matches: Vec<String> = tools
        .iter()
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

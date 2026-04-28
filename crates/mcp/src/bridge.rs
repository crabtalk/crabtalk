//! Crabtalk MCP bridge — connects to MCP servers and routes tool calls.
//!
//! Peers are keyed by an opaque `peer_id` chosen by the caller — today
//! `McpHandler` passes the fingerprint hex of the config so identical
//! configs share one peer. The bridge itself doesn't interpret the id;
//! it just stores peers and routes calls.

use crate::client::{self, McpPeer};
use anyhow::Result;
use tokio::sync::Mutex;
use wcore::model::Tool;

/// A connected MCP server peer with its tool definitions.
struct ConnectedPeer {
    id: String,
    peer: McpPeer,
    tools: Vec<Tool>,
}

/// Bridge to one or more MCP servers.
pub struct McpBridge {
    peers: Mutex<Vec<ConnectedPeer>>,
}

impl Default for McpBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl McpBridge {
    /// Create a new empty bridge with no connected peers.
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(Vec::new()),
        }
    }

    /// Connect to a peer by spawning a child process.
    pub async fn connect_stdio_named(
        &self,
        id: String,
        command: tokio::process::Command,
    ) -> Result<Vec<String>> {
        self.register_peer(id, McpPeer::stdio(command)?).await
    }

    /// Connect to a peer over HTTP transport.
    pub async fn connect_http_named(&self, id: String, url: &str) -> Result<Vec<String>> {
        self.register_peer(id, McpPeer::http(url)).await
    }

    /// Initialize a peer, fetch its tools, and store it.
    async fn register_peer(&self, id: String, mut peer: McpPeer) -> Result<Vec<String>> {
        peer.initialize().await?;
        let mcp_tools = peer.list_all_tools().await?;
        let tools: Vec<Tool> = mcp_tools.iter().map(convert_tool).collect();
        let tool_names: Vec<String> = tools.iter().map(|t| t.function.name.clone()).collect();
        self.peers
            .lock()
            .await
            .push(ConnectedPeer { id, peer, tools });
        Ok(tool_names)
    }

    /// Remove a peer by id, returning the tool names that were dropped.
    pub async fn remove_server(&self, id: &str) -> Vec<String> {
        let mut peers = self.peers.lock().await;
        let mut removed = Vec::new();
        peers.retain(|p| {
            if p.id == id {
                removed.extend(p.tools.iter().map(|t| t.function.name.clone()));
                false
            } else {
                true
            }
        });
        removed
    }

    /// Tool definitions exposed by the listed peers, in the order they
    /// were declared. Used by the dispatcher to render the per-agent
    /// fuzzy listing without exposing tools the agent didn't ask for.
    pub async fn tools_for(&self, peer_ids: &[String]) -> Vec<Tool> {
        let peers = self.peers.lock().await;
        let mut out: Vec<Tool> = Vec::new();
        for id in peer_ids {
            if let Some(peer) = peers.iter().find(|p| &p.id == id) {
                out.extend(peer.tools.iter().cloned());
            }
        }
        out
    }

    /// Call a tool on the named peer.
    ///
    /// Routing is by `(peer_id, tool_name)` — two peers may export tools
    /// with the same name without collision.
    pub async fn call(
        &self,
        peer_id: &str,
        tool_name: &str,
        arguments: &str,
    ) -> Result<String, String> {
        let mut peers = self.peers.lock().await;
        let Some(peer) = peers.iter_mut().find(|p| p.id == peer_id) else {
            return Err(format!("mcp peer '{peer_id}' not connected"));
        };
        if !peer.tools.iter().any(|t| t.function.name == tool_name) {
            return Err(format!(
                "mcp tool '{tool_name}' not exported by peer '{peer_id}'"
            ));
        }

        let args: Option<serde_json::Map<String, serde_json::Value>> = if arguments.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(arguments)
                    .map_err(|e| format!("invalid tool arguments: {e}"))?,
            )
        };

        match peer.peer.call_tool(tool_name, args).await {
            Ok(result) => {
                let text = extract_text(&result.content);
                if result.is_error == Some(true) {
                    Err(format!("mcp tool error: {text}"))
                } else {
                    Ok(text)
                }
            }
            Err(e) => Err(format!("mcp call failed: {e}")),
        }
    }
}

/// Convert an MCP tool to a `crabllm_core::Tool` envelope.
fn convert_tool(mcp_tool: &client::McpTool) -> Tool {
    use wcore::model::{FunctionDef, ToolType};
    Tool {
        kind: ToolType::Function,
        function: FunctionDef {
            name: mcp_tool.name.clone(),
            description: mcp_tool.description.clone(),
            parameters: mcp_tool.input_schema.clone(),
        },
        strict: None,
    }
}

/// Extract text content from MCP Content items.
fn extract_text(content: &[client::ContentItem]) -> String {
    content
        .iter()
        .filter(|c| c.content_type == "text")
        .filter_map(|c| c.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n")
}

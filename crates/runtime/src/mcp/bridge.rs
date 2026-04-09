//! Crabtalk MCP bridge — connects to MCP servers and dispatches tool calls.

use crate::mcp::client::{self, McpPeer};
use anyhow::Result;
use std::collections::BTreeMap;
use tokio::sync::Mutex;
use wcore::model::Tool;

/// A connected MCP server peer with its tool names.
struct ConnectedPeer {
    name: String,
    peer: McpPeer,
    tools: Vec<String>,
}

/// Bridge to one or more MCP servers.
pub struct McpBridge {
    peers: Mutex<Vec<ConnectedPeer>>,
    /// Cache of converted tools keyed by name.
    tool_cache: Mutex<BTreeMap<String, Tool>>,
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
            tool_cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Connect to an MCP server by spawning a child process.
    pub async fn connect_stdio(&self, command: tokio::process::Command) -> Result<()> {
        let name = command
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned();
        self.connect_stdio_named(name, command).await?;
        Ok(())
    }

    /// Connect to a named MCP server by spawning a child process.
    pub async fn connect_stdio_named(
        &self,
        name: String,
        command: tokio::process::Command,
    ) -> Result<Vec<String>> {
        self.register_peer(name, McpPeer::stdio(command)?).await
    }

    /// Connect to a named MCP server via HTTP transport.
    pub async fn connect_http_named(&self, name: String, url: &str) -> Result<Vec<String>> {
        self.register_peer(name, McpPeer::http(url)).await
    }

    /// Initialize a peer, register its tools, and store it.
    async fn register_peer(&self, name: String, mut peer: McpPeer) -> Result<Vec<String>> {
        peer.initialize().await?;
        let mcp_tools = peer.list_all_tools().await?;

        let mut tool_names = Vec::with_capacity(mcp_tools.len());
        {
            let mut cache = self.tool_cache.lock().await;
            for mcp_tool in &mcp_tools {
                let ct_tool = convert_tool(mcp_tool);
                let tool_name = ct_tool.function.name.clone();
                use std::collections::btree_map::Entry;
                match cache.entry(tool_name.clone()) {
                    Entry::Occupied(_) => tracing::warn!(
                        "MCP tool '{}' from server '{}' conflicts with already-registered tool, skipping",
                        tool_name,
                        name
                    ),
                    Entry::Vacant(e) => {
                        tool_names.push(tool_name);
                        e.insert(ct_tool);
                    }
                }
            }
        }

        self.peers.lock().await.push(ConnectedPeer {
            name,
            peer,
            tools: tool_names.clone(),
        });

        Ok(tool_names)
    }

    /// Disconnect all peers and clear the tool cache.
    pub async fn clear(&self) {
        self.peers.lock().await.clear();
        self.tool_cache.lock().await.clear();
    }

    /// Remove a server by name, returning the tool names that were removed.
    pub async fn remove_server(&self, name: &str) -> Vec<String> {
        let mut peers = self.peers.lock().await;
        let mut removed_tools = Vec::new();

        peers.retain(|p| {
            if p.name.as_str() == name {
                removed_tools.extend(p.tools.iter().cloned());
                false
            } else {
                true
            }
        });

        let mut cache = self.tool_cache.lock().await;
        for tool_name in &removed_tools {
            cache.remove(tool_name);
        }

        removed_tools
    }

    /// List all connected servers with their tool names.
    pub async fn list_servers(&self) -> Vec<(String, Vec<String>)> {
        self.peers
            .lock()
            .await
            .iter()
            .map(|p| (p.name.clone(), p.tools.clone()))
            .collect()
    }

    /// List all tools available across all connected peers.
    pub async fn tools(&self) -> Vec<Tool> {
        self.tool_cache.lock().await.values().cloned().collect()
    }

    /// Try to list tools without blocking. Returns empty if the lock is held.
    pub fn try_tools(&self) -> Vec<Tool> {
        self.tool_cache
            .try_lock()
            .map(|cache| cache.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Call a tool by name, routing to the correct peer.
    ///
    /// Returns `Ok` on success, `Err` for routing failures, arg parse
    /// errors, MCP-reported tool errors, or transport failures.
    pub async fn call(&self, name: &str, arguments: &str) -> Result<String, String> {
        let mut peers = self.peers.lock().await;
        let connected = peers
            .iter_mut()
            .find(|p| p.tools.iter().any(|t| t.as_str() == name));

        let Some(connected) = connected else {
            return Err(format!("mcp tool '{name}' not available"));
        };

        let args: Option<serde_json::Map<String, serde_json::Value>> = if arguments.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(arguments)
                    .map_err(|e| format!("invalid tool arguments: {e}"))?,
            )
        };

        match connected.peer.call_tool(name, args).await {
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

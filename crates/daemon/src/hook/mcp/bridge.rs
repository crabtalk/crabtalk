//! //! Walrus MCP bridge — connects to MCP servers and dispatches tool calls.

use anyhow::Result;
use compact_str::CompactString;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, RawContent},
    service::{RoleClient, RunningService},
    transport::TokioChildProcess,
};
use std::collections::BTreeMap;
use tokio::sync::Mutex;
use wcore::model::Tool;

/// A connected MCP server peer with its tool names.
struct ConnectedPeer {
    name: CompactString,
    peer: RunningService<RoleClient, ()>,
    tools: Vec<CompactString>,
}

/// Bridge to one or more MCP servers via the rmcp SDK.
///
/// Converts MCP tool definitions to walrus-core [`Tool`] schemas and
/// dispatches tool calls through the protocol.
pub struct McpBridge {
    peers: Mutex<Vec<ConnectedPeer>>,
    /// Cache of converted tools keyed by name.
    tool_cache: Mutex<BTreeMap<CompactString, Tool>>,
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
        self.connect_stdio_named(CompactString::from(name), command)
            .await?;
        Ok(())
    }

    /// Connect to a named MCP server by spawning a child process.
    ///
    /// Returns the list of tool names registered by this server.
    pub async fn connect_stdio_named(
        &self,
        name: CompactString,
        command: tokio::process::Command,
    ) -> Result<Vec<CompactString>> {
        let transport = TokioChildProcess::new(command)?;
        let peer: RunningService<RoleClient, ()> = ().serve(transport).await?;

        let mcp_tools = peer.list_all_tools().await?;
        let mut tool_names = Vec::with_capacity(mcp_tools.len());

        {
            let mut cache = self.tool_cache.lock().await;
            for mcp_tool in &mcp_tools {
                let walrus_tool = self::convert_tool(mcp_tool);
                tool_names.push(walrus_tool.name.clone());
                cache.insert(walrus_tool.name.clone(), walrus_tool);
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
    pub async fn remove_server(&self, name: &str) -> Vec<CompactString> {
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
    pub async fn list_servers(&self) -> Vec<(CompactString, Vec<CompactString>)> {
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
    pub async fn call(&self, name: &str, arguments: &str) -> String {
        let peers = self.peers.lock().await;
        let connected = peers
            .iter()
            .find(|p| p.tools.iter().any(|t| t.as_str() == name));

        let Some(connected) = connected else {
            return format!("mcp tool '{name}' not available");
        };

        let args: Option<serde_json::Map<String, serde_json::Value>> = if arguments.is_empty() {
            None
        } else {
            match serde_json::from_str(arguments) {
                Ok(v) => Some(v),
                Err(e) => return format!("invalid tool arguments: {e}"),
            }
        };

        let params = CallToolRequestParams {
            meta: None,
            name: name.to_string().into(),
            arguments: args,
            task: None,
        };

        match connected.peer.call_tool(params).await {
            Ok(result) => {
                if result.is_error == Some(true) {
                    format!("mcp tool error: {}", self::extract_text(&result.content))
                } else {
                    self::extract_text(&result.content)
                }
            }
            Err(e) => format!("mcp call failed: {e}"),
        }
    }
}

/// Convert an rmcp Tool to a walrus-core Tool.
fn convert_tool(mcp_tool: &rmcp::model::Tool) -> Tool {
    let schema_value =
        serde_json::to_value(mcp_tool.input_schema.as_ref()).unwrap_or(serde_json::json!({}));
    let parameters: schemars::Schema =
        serde_json::from_value(schema_value).unwrap_or_else(|_| schemars::schema_for!(String));

    Tool {
        name: CompactString::from(mcp_tool.name.as_ref()),
        description: mcp_tool
            .description
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_default(),
        parameters,
        strict: false,
    }
}

/// Extract text content from MCP Content items.
fn extract_text(content: &[rmcp::model::Content]) -> String {
    content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

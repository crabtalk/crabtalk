//! MCP bridge: connects to MCP servers and converts tools/calls.
//!
//! The [`McpBridge`] holds connected MCP server peers and provides
//! tool listing and call dispatch through the MCP protocol.

use anyhow::Result;
use compact_str::CompactString;
use llm::Tool;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, RawContent},
    service::{RoleClient, RunningService},
    transport::TokioChildProcess,
};
use std::collections::BTreeMap;
use tokio::{process::Command, sync::Mutex};

/// A connected MCP server peer with its tool names.
struct ConnectedPeer {
    peer: RunningService<RoleClient, ()>,
    tools: Vec<CompactString>,
}

/// Bridge to one or more MCP servers via the rmcp SDK.
///
/// Converts MCP tool definitions to walrus-llm [`Tool`] schemas and
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
    ///
    /// The command should be a program that speaks MCP over stdio.
    pub async fn connect_stdio(&self, command: Command) -> Result<()> {
        let transport = TokioChildProcess::new(command)?;
        let peer: RunningService<RoleClient, ()> = ().serve(transport).await?;

        let mcp_tools = peer.list_all_tools().await?;
        let mut tool_names = Vec::with_capacity(mcp_tools.len());

        {
            let mut cache = self.tool_cache.lock().await;
            for mcp_tool in &mcp_tools {
                let walrus_tool = convert_tool(mcp_tool);
                tool_names.push(walrus_tool.name.clone());
                cache.insert(walrus_tool.name.clone(), walrus_tool);
            }
        }

        self.peers.lock().await.push(ConnectedPeer {
            peer,
            tools: tool_names,
        });

        Ok(())
    }

    /// List all tools available across all connected peers.
    pub async fn tools(&self) -> Vec<Tool> {
        self.tool_cache.lock().await.values().cloned().collect()
    }

    /// Call a tool by name, routing to the correct peer.
    ///
    /// Returns the tool output as a String. If the tool is not found
    /// or the call fails, returns an error description.
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
                    format!("mcp tool error: {}", extract_text(&result.content))
                } else {
                    extract_text(&result.content)
                }
            }
            Err(e) => format!("mcp call failed: {e}"),
        }
    }
}

/// Convert an rmcp Tool to a walrus-llm Tool.
pub fn convert_tool(mcp_tool: &rmcp::model::Tool) -> Tool {
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

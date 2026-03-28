//! Minimal MCP client — JSON-RPC 2.0 over stdio or HTTP.
//!
//! Only supports the three methods crabtalk actually uses:
//! `initialize`, `tools/list`, and `tools/call`.

use anyhow::{Context, Result};

pub use jsonrpc::{CallToolResult, ContentItem, McpTool};
use jsonrpc::{ClientInfo, InitializeParams, ListToolsResult};

mod http;
mod jsonrpc;
mod stdio;

pub enum McpPeer {
    Stdio(stdio::StdioTransport),
    Http(http::HttpTransport),
}

impl McpPeer {
    pub fn stdio(command: tokio::process::Command) -> Result<Self> {
        Ok(Self::Stdio(stdio::StdioTransport::new(command)?))
    }

    pub fn http(url: &str) -> Self {
        Self::Http(http::HttpTransport::new(url))
    }

    async fn request(&mut self, msg: serde_json::Value) -> Result<serde_json::Value> {
        match self {
            Self::Stdio(t) => t.request(msg).await,
            Self::Http(t) => t.request(msg).await,
        }
    }

    async fn notify(&mut self, msg: serde_json::Value) -> Result<()> {
        match self {
            Self::Stdio(t) => t.notify(msg).await,
            Self::Http(t) => t.notify(msg).await,
        }
    }

    /// Run the MCP initialization handshake.
    pub async fn initialize(&mut self) -> Result<()> {
        let params = InitializeParams {
            protocol_version: "2025-03-26",
            capabilities: serde_json::json!({}),
            client_info: ClientInfo {
                name: "crabtalk",
                version: env!("CARGO_PKG_VERSION"),
            },
        };

        let req = jsonrpc::request("initialize", serde_json::to_value(params)?);
        let resp = self.request(req).await?;
        let _ = jsonrpc::extract_result(resp)?;

        self.notify(jsonrpc::notification("notifications/initialized"))
            .await?;
        Ok(())
    }

    /// List all tools, following pagination cursors.
    pub async fn list_all_tools(&mut self) -> Result<Vec<McpTool>> {
        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = match &cursor {
                Some(c) => serde_json::json!({ "cursor": c }),
                None => serde_json::json!({}),
            };

            let req = jsonrpc::request("tools/list", params);
            let resp = self.request(req).await?;
            let result = jsonrpc::extract_result(resp)?;
            let list: ListToolsResult =
                serde_json::from_value(result).context("failed to parse tools/list response")?;

            all_tools.extend(list.tools);

            match list.next_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }

        Ok(all_tools)
    }

    /// Call a tool by name with optional JSON arguments.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult> {
        let mut params = serde_json::json!({ "name": name });
        if let Some(args) = arguments {
            params["arguments"] = serde_json::Value::Object(args);
        }

        let req = jsonrpc::request("tools/call", params);
        let resp = self.request(req).await?;
        let result = jsonrpc::extract_result(resp)?;
        serde_json::from_value(result).context("failed to parse tools/call response")
    }
}

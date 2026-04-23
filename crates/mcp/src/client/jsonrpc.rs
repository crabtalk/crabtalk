//! JSON-RPC 2.0 framing helpers and MCP response types.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn request(method: &str, params: serde_json::Value) -> serde_json::Value {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

pub fn notification(method: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
    })
}

pub fn extract_result(response: serde_json::Value) -> Result<serde_json::Value> {
    if let Some(err) = response.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        bail!("JSON-RPC error {code}: {msg}");
    }
    response
        .get("result")
        .cloned()
        .context("missing 'result' in JSON-RPC response")
}

// ── MCP response types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallToolResult {
    pub content: Vec<ContentItem>,
    #[serde(rename = "isError")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: Option<String>,
}

#[derive(Serialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: &'static str,
    pub capabilities: serde_json::Value,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

#[derive(Serialize)]
pub struct ClientInfo {
    pub name: &'static str,
    pub version: &'static str,
}

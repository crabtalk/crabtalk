//! Mock MCP server — scriptable tool responses over JSON-RPC 2.0.
//!
//! Implements the three MCP methods (`initialize`, `tools/list`, `tools/call`)
//! as a plain axum server. No rmcp dependency needed.

use crate::task::Task;
use axum::{Router, extract::State, http::StatusCode, routing::post};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::net::TcpListener;

/// A recorded tool call for metrics.
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool: String,
    pub args: Value,
    pub timestamp: Instant,
}

struct MockState {
    tools: Value,
    responses: HashMap<String, Vec<ResponseEntry>>,
    call_counts: Mutex<HashMap<String, usize>>,
    records: Arc<Mutex<Vec<ToolCallRecord>>>,
}

struct ResponseEntry {
    output: String,
    is_error: bool,
}

/// Handle to a running mock MCP server.
pub struct MockMcpHandle {
    addr: SocketAddr,
    records: Arc<Mutex<Vec<ToolCallRecord>>>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl MockMcpHandle {
    /// Address the server is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Collected tool call records.
    pub fn metrics(&self) -> Vec<ToolCallRecord> {
        self.records.lock().unwrap().clone()
    }

    /// Reset metrics between benchmark iterations.
    pub fn reset_metrics(&self) {
        self.records.lock().unwrap().clear();
    }

    /// Shut down the server.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
    }
}

/// Start the mock MCP server on a random port.
pub async fn start(tasks: &[Task]) -> (SocketAddr, MockMcpHandle) {
    let mut tool_schemas = Vec::new();
    let mut responses: HashMap<String, Vec<ResponseEntry>> = HashMap::new();

    for task in tasks {
        for tool in &task.tools {
            // Deduplicate tools by name.
            if !tool_schemas.iter().any(|t: &Value| t["name"] == tool.name) {
                tool_schemas.push(json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.parameters,
                }));
            }
        }
        for resp in &task.responses {
            responses
                .entry(resp.tool.to_string())
                .or_default()
                .push(ResponseEntry {
                    output: resp.output.to_string(),
                    is_error: resp.is_error,
                });
        }
    }

    let records = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(MockState {
        tools: json!({ "tools": tool_schemas }),
        responses,
        call_counts: Mutex::new(HashMap::new()),
        records: Arc::clone(&records),
    });

    let app = Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let handle = MockMcpHandle {
        addr,
        records,
        shutdown: shutdown_tx,
    };
    (addr, handle)
}

async fn handle_mcp(
    State(state): State<Arc<MockState>>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    let id = body.get("id").cloned().unwrap_or(json!(1));
    let method = body["method"].as_str().unwrap_or("");

    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "serverInfo": { "name": "mock-mcp", "version": "0.1.0" },
            "capabilities": { "tools": {} }
        }),

        "tools/list" => state.tools.clone(),

        "tools/call" => {
            let name = body["params"]["name"].as_str().unwrap_or("");
            let args = body["params"]["arguments"].clone();

            // Record the call.
            state.records.lock().unwrap().push(ToolCallRecord {
                tool: name.to_string(),
                args: args.clone(),
                timestamp: Instant::now(),
            });

            // Get the next scripted response for this tool.
            let mut counts = state.call_counts.lock().unwrap();
            let idx = counts.entry(name.to_string()).or_insert(0);
            let entry = state
                .responses
                .get(name)
                .and_then(|v| v.get(*idx).or_else(|| v.last()));
            *idx += 1;

            match entry {
                Some(r) => json!({
                    "content": [{ "type": "text", "text": r.output }],
                    "isError": r.is_error
                }),
                None => json!({
                    "content": [{ "type": "text", "text": format!("mock: no response configured for tool '{name}'") }],
                    "isError": true
                }),
            }
        }

        _ => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("unknown method: {method}") }
                })),
            );
        }
    };

    (
        StatusCode::OK,
        axum::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })),
    )
}

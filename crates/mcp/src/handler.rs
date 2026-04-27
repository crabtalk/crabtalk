//! Crabtalk MCP handler — initial load and read access.

use crate::McpBridge;
use parking_lot::RwLock as SyncRwLock;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;
use wcore::McpServerConfig;

/// Connection status for a single registered MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    /// Connect attempt in progress.
    Connecting,
    /// Connected and tools registered.
    Connected,
    /// Connect attempt failed; see `last_error` on the state.
    Failed,
    /// Previously connected, now removed (kept on the map for diagnostics).
    Disconnected,
}

/// Per-server lifecycle state mirrored on the handler.
///
/// The bridge owns the live peer set; this map is the queryable view —
/// it retains failures (which the bridge drops) and surfaces "currently
/// trying" between attempts.
#[derive(Debug, Clone)]
pub struct McpServerState {
    pub status: ServerStatus,
    pub tools: Vec<String>,
    pub last_error: Option<String>,
}

impl McpServerState {
    fn connecting() -> Self {
        Self {
            status: ServerStatus::Connecting,
            tools: Vec::new(),
            last_error: None,
        }
    }

    fn connected(tools: Vec<String>) -> Self {
        Self {
            status: ServerStatus::Connected,
            tools,
            last_error: None,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            status: ServerStatus::Failed,
            tools: Vec::new(),
            last_error: Some(error),
        }
    }
}

/// MCP bridge owner.
pub struct McpHandler {
    bridge: RwLock<Arc<McpBridge>>,
    /// Per-server state, keyed by server name.
    states: SyncRwLock<BTreeMap<String, McpServerState>>,
}

impl McpHandler {
    /// Create an empty handler with no connected servers.
    pub fn empty() -> Self {
        Self {
            bridge: RwLock::new(Arc::new(McpBridge::new())),
            states: SyncRwLock::new(BTreeMap::new()),
        }
    }

    /// Build a bridge from the given MCP server configs and discovered port files.
    /// Timeout for connecting to a single MCP server (30 seconds).
    const MCP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    async fn build_bridge(
        configs: &[McpServerConfig],
    ) -> (McpBridge, BTreeMap<String, McpServerState>) {
        let bridge = McpBridge::new();
        let mut states: BTreeMap<String, McpServerState> = BTreeMap::new();

        // 1. Connect servers from config.
        for server_config in configs {
            states.insert(server_config.name.clone(), McpServerState::connecting());
            let state = connect_one(&bridge, server_config).await;
            states.insert(server_config.name.clone(), state);
        }

        // 2. Auto-discover services from port files not already registered.
        for (name, url) in scan_port_files() {
            if states.contains_key(&name) {
                continue;
            }
            states.insert(name.clone(), McpServerState::connecting());
            let state = connect_http(&bridge, &name, &url).await;
            states.insert(name, state);
        }

        (bridge, states)
    }

    /// Load MCP servers from the given configs at startup.
    pub async fn load(configs: &[McpServerConfig]) -> Self {
        let (bridge, states) = Self::build_bridge(configs).await;
        Self {
            bridge: RwLock::new(Arc::new(bridge)),
            states: SyncRwLock::new(states),
        }
    }

    /// List all connected servers with their tool names (live, from the bridge).
    pub async fn list(&self) -> Vec<(String, Vec<String>)> {
        self.bridge.read().await.list_servers().await
    }

    /// Sync access to the cached list of *connected* servers and their tools.
    pub fn cached_list(&self) -> Vec<(String, Vec<String>)> {
        self.states
            .read()
            .iter()
            .filter(|(_, s)| s.status == ServerStatus::Connected)
            .map(|(name, s)| (name.clone(), s.tools.clone()))
            .collect()
    }

    /// Snapshot of every registered server's state (connected, failed, or other).
    pub fn states(&self) -> BTreeMap<String, McpServerState> {
        self.states.read().clone()
    }

    /// Get a clone of the current bridge Arc.
    pub async fn bridge(&self) -> Arc<McpBridge> {
        Arc::clone(&*self.bridge.read().await)
    }

    /// Try to get a clone of the current bridge Arc without blocking.
    pub fn try_bridge(&self) -> Option<Arc<McpBridge>> {
        self.bridge.try_read().ok().map(|g| Arc::clone(&*g))
    }
}

/// Attempt to connect a single server, applying the global timeout.
async fn connect_one(bridge: &McpBridge, cfg: &McpServerConfig) -> McpServerState {
    let fut = async {
        if let Some(url) = &cfg.url {
            tracing::info!(server = %cfg.name, %url, "connecting MCP server via HTTP");
            bridge.connect_http_named(cfg.name.clone(), url).await
        } else {
            let mut cmd = tokio::process::Command::new(&cfg.command);
            cmd.args(&cfg.args);
            for (k, v) in &cfg.env {
                cmd.env(k, v);
            }
            tracing::info!(
                server = %cfg.name,
                command = %cfg.command,
                "connecting MCP server via stdio"
            );
            bridge.connect_stdio_named(cfg.name.clone(), cmd).await
        }
    };

    match tokio::time::timeout(McpHandler::MCP_CONNECT_TIMEOUT, fut).await {
        Ok(Ok(tools)) => {
            tracing::info!(
                "connected MCP server '{}' — {} tool(s)",
                cfg.name,
                tools.len()
            );
            McpServerState::connected(tools)
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            tracing::warn!("failed to connect MCP server '{}': {msg}", cfg.name);
            McpServerState::failed(msg)
        }
        Err(_) => {
            let msg = format!(
                "timed out after {}s",
                McpHandler::MCP_CONNECT_TIMEOUT.as_secs()
            );
            tracing::warn!("MCP server '{}' {msg}, skipping", cfg.name);
            McpServerState::failed(msg)
        }
    }
}

/// Attempt to connect via an already-known HTTP URL (port-file discovery).
async fn connect_http(bridge: &McpBridge, name: &str, url: &str) -> McpServerState {
    tracing::info!(server = %name, %url, "connecting MCP server via port file");
    match tokio::time::timeout(
        McpHandler::MCP_CONNECT_TIMEOUT,
        bridge.connect_http_named(name.to_string(), url),
    )
    .await
    {
        Ok(Ok(tools)) => {
            tracing::info!("connected MCP server '{name}' — {} tool(s)", tools.len());
            McpServerState::connected(tools)
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            tracing::warn!("failed to connect MCP server '{name}': {msg}");
            McpServerState::failed(msg)
        }
        Err(_) => {
            let msg = format!(
                "timed out after {}s",
                McpHandler::MCP_CONNECT_TIMEOUT.as_secs()
            );
            tracing::warn!("MCP server '{name}' {msg}, skipping");
            McpServerState::failed(msg)
        }
    }
}

/// Scan `~/.crabtalk/run/*.port` for service port files.
fn scan_port_files() -> Vec<(String, String)> {
    let run_dir = &*wcore::paths::RUN_DIR;
    let entries = match std::fs::read_dir(run_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension() else {
            continue;
        };
        if ext != "port" {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip the daemon's own port file.
        if stem == "crabtalk" {
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(port) = contents.trim().parse::<u16>()
        {
            result.push((stem.to_string(), format!("http://127.0.0.1:{port}/mcp")));
        }
    }
    result
}

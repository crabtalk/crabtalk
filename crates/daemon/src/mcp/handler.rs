//! Crabtalk MCP handler — initial load and read access.

use crate::mcp::{McpBridge, config::McpServerConfig};
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::sync::RwLock;

/// MCP bridge owner.
pub struct McpHandler {
    bridge: RwLock<Arc<McpBridge>>,
    /// Sync cache of server names → tool names, populated at load/reload.
    server_cache: StdRwLock<Vec<(String, Vec<String>)>>,
}

impl McpHandler {
    /// Create an empty handler with no connected servers.
    pub fn empty() -> Self {
        Self {
            bridge: RwLock::new(Arc::new(McpBridge::new())),
            server_cache: StdRwLock::new(Vec::new()),
        }
    }

    /// Build a bridge from the given MCP server configs and discovered port files.
    /// Timeout for connecting to a single MCP server (30 seconds).
    const MCP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    async fn build_bridge(configs: &[McpServerConfig]) -> McpBridge {
        let bridge = McpBridge::new();
        let mut connected_names: Vec<String> = Vec::new();

        // 1. Connect servers from config.
        for server_config in configs {
            let fut = async {
                if let Some(url) = &server_config.url {
                    tracing::info!(
                        server = %server_config.name,
                        url = %url,
                        "connecting MCP server via HTTP"
                    );
                    bridge
                        .connect_http_named(server_config.name.clone(), url)
                        .await
                } else {
                    let mut cmd = tokio::process::Command::new(&server_config.command);
                    cmd.args(&server_config.args);
                    for (k, v) in &server_config.env {
                        cmd.env(k, v);
                    }
                    tracing::info!(
                        server = %server_config.name,
                        command = %server_config.command,
                        "connecting MCP server via stdio"
                    );
                    bridge
                        .connect_stdio_named(server_config.name.clone(), cmd)
                        .await
                }
            };

            match tokio::time::timeout(Self::MCP_CONNECT_TIMEOUT, fut).await {
                Ok(Ok(tools)) => {
                    connected_names.push(server_config.name.clone());
                    tracing::info!(
                        "connected MCP server '{}' — {} tool(s)",
                        server_config.name,
                        tools.len()
                    );
                }
                Ok(Err(e)) => {
                    tracing::warn!("failed to connect MCP server '{}': {e}", server_config.name);
                }
                Err(_) => {
                    tracing::warn!(
                        "MCP server '{}' timed out after {}s, skipping",
                        server_config.name,
                        Self::MCP_CONNECT_TIMEOUT.as_secs()
                    );
                }
            }
        }

        // 2. Auto-discover services from port files not already connected.
        for (name, url) in scan_port_files() {
            if connected_names.iter().any(|n| n == &name) {
                continue;
            }
            tracing::info!(
                server = %name,
                url = %url,
                "connecting MCP server via port file"
            );
            match tokio::time::timeout(
                Self::MCP_CONNECT_TIMEOUT,
                bridge.connect_http_named(name.clone(), &url),
            )
            .await
            {
                Ok(Ok(tools)) => {
                    tracing::info!("connected MCP server '{name}' — {} tool(s)", tools.len());
                }
                Ok(Err(e)) => {
                    tracing::warn!("failed to connect MCP server '{name}': {e}");
                }
                Err(_) => {
                    tracing::warn!(
                        "MCP server '{name}' timed out after {}s, skipping",
                        Self::MCP_CONNECT_TIMEOUT.as_secs()
                    );
                }
            }
        }

        bridge
    }

    /// Load MCP servers from the given configs at startup.
    pub async fn load(configs: &[McpServerConfig]) -> Self {
        let bridge = Self::build_bridge(configs).await;
        let servers = bridge.list_servers().await;
        Self {
            bridge: RwLock::new(Arc::new(bridge)),
            server_cache: StdRwLock::new(servers),
        }
    }

    /// List all connected servers with their tool names.
    pub async fn list(&self) -> Vec<(String, Vec<String>)> {
        self.bridge.read().await.list_servers().await
    }

    /// Sync access to the cached server→tools list (populated at load time).
    pub fn cached_list(&self) -> Vec<(String, Vec<String>)> {
        self.server_cache.read().unwrap().clone()
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

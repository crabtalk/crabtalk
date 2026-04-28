//! Crabtalk MCP handler — initial load, mutation, state, and events.

use crate::McpBridge;
use parking_lot::RwLock as SyncRwLock;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{RwLock, broadcast};
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

/// Lifecycle event emitted on every state transition.
///
/// Events go to a `tokio::sync::broadcast` channel — late subscribers
/// see only events emitted after they subscribe. To recover prior state,
/// callers should pair `subscribe()` with a fresh `states()` snapshot.
#[derive(Debug, Clone)]
pub enum McpEvent {
    Connecting { name: String },
    Connected { name: String, tools: Vec<String> },
    Failed { name: String, error: String },
    Disconnected { name: String },
}

const EVENT_CHANNEL_CAPACITY: usize = 256;

/// MCP bridge owner.
pub struct McpHandler {
    bridge: RwLock<Arc<McpBridge>>,
    /// Per-server state, keyed by server name.
    states: SyncRwLock<BTreeMap<String, McpServerState>>,
    events_tx: broadcast::Sender<McpEvent>,
}

impl McpHandler {
    /// Timeout for connecting to a single MCP server (30 seconds).
    const MCP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    /// Create an empty handler with no connected servers.
    pub fn empty() -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            bridge: RwLock::new(Arc::new(McpBridge::new())),
            states: SyncRwLock::new(BTreeMap::new()),
            events_tx,
        }
    }

    /// Load MCP servers from the given configs at startup.
    pub async fn load(configs: &[McpServerConfig]) -> Self {
        let handler = Self::empty();
        handler.connect_initial(configs).await;
        handler
    }

    async fn connect_initial(&self, configs: &[McpServerConfig]) {
        for cfg in configs {
            self.upsert_server(cfg).await;
        }
        for (name, url) in scan_port_files() {
            if self.states.read().contains_key(&name) {
                continue;
            }
            let cfg = McpServerConfig {
                name,
                url: Some(url),
                ..Default::default()
            };
            self.upsert_server(&cfg).await;
        }
    }

    /// Subscribe to lifecycle events. The returned receiver yields every
    /// transition emitted while it is alive.
    pub fn subscribe(&self) -> broadcast::Receiver<McpEvent> {
        self.events_tx.subscribe()
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

    /// Connect (or reconnect) a single server in place.
    ///
    /// Removes any prior peer for this name from the bridge, marks the
    /// state as `Connecting`, attempts a fresh connect, and stores the
    /// resulting state. Emits a `Connecting` event followed by
    /// `Connected` or `Failed`. Returns the new state.
    pub async fn upsert_server(&self, cfg: &McpServerConfig) -> McpServerState {
        let bridge = self.bridge().await;
        bridge.remove_server(&cfg.name).await;
        self.set_state(&cfg.name, McpServerState::connecting());
        self.emit(McpEvent::Connecting {
            name: cfg.name.clone(),
        });

        let state = connect_one(&bridge, cfg).await;
        self.set_state(&cfg.name, state.clone());
        match &state.status {
            ServerStatus::Connected => self.emit(McpEvent::Connected {
                name: cfg.name.clone(),
                tools: state.tools.clone(),
            }),
            ServerStatus::Failed => self.emit(McpEvent::Failed {
                name: cfg.name.clone(),
                error: state.last_error.clone().unwrap_or_default(),
            }),
            ServerStatus::Connecting | ServerStatus::Disconnected => {}
        }
        state
    }

    /// Disconnect and forget a single server.
    ///
    /// Returns the prior state, if any. Emits `Disconnected` only when
    /// an entry actually existed.
    pub async fn disconnect_server(&self, name: &str) -> Option<McpServerState> {
        let bridge = self.bridge().await;
        bridge.remove_server(name).await;
        let prior = self.states.write().remove(name);
        if prior.is_some() {
            self.emit(McpEvent::Disconnected {
                name: name.to_string(),
            });
        }
        prior
    }

    fn set_state(&self, name: &str, state: McpServerState) {
        self.states.write().insert(name.to_string(), state);
    }

    fn emit(&self, event: McpEvent) {
        let _ = self.events_tx.send(event);
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

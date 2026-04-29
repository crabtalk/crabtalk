//! Crabtalk MCP handler — agent-driven registration with fingerprint-keyed dedup.
//!
//! Agents declare their MCP servers inline (RFC 0193). The handler tracks
//! which agents have declared which configs and dedups identical configs
//! by structural fingerprint — two agents declaring the exact same
//! `(command, args, env, url)` share one peer process. The peer survives
//! until the last agent referencing it unregisters.

use crate::McpBridge;
use parking_lot::RwLock as SyncRwLock;
use std::{
    collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};
use tokio::sync::{RwLock, broadcast};
use wcore::McpServerConfig;

/// Stable identifier for a peer process — hash of the structural config.
/// Two configs with the same fingerprint produce the same peer; different
/// fingerprints get separate peers.
pub type Fingerprint = u64;

/// Compute the dedup fingerprint for a config. Hashes the fields that
/// affect process identity: command, args, env, and url. `name` and
/// `auto_restart` are not part of the fingerprint — they are
/// presentation-level.
pub fn fingerprint(cfg: &McpServerConfig) -> Fingerprint {
    let mut h = DefaultHasher::new();
    cfg.command.hash(&mut h);
    cfg.args.hash(&mut h);
    // BTreeMap iterates in sorted order — fingerprint is order-independent.
    for (k, v) in &cfg.env {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    cfg.url.hash(&mut h);
    h.finish()
}

/// Connection status for a single peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Connecting,
    Connected,
    Failed,
    Disconnected,
}

/// Per-peer lifecycle state mirrored on the handler.
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

/// One peer's tracked state plus the (agent, name) pairs that own it.
#[derive(Debug)]
struct PeerEntry {
    state: McpServerState,
    /// Owners — at least one. When this drops to empty the peer is torn down.
    refs: BTreeSet<(String, String)>,
}

/// Lifecycle event emitted on every state transition.
#[derive(Debug, Clone)]
pub enum McpEvent {
    Connecting {
        agent: String,
        name: String,
    },
    Connected {
        agent: String,
        name: String,
        tools: Vec<String>,
    },
    Failed {
        agent: String,
        name: String,
        error: String,
    },
    Disconnected {
        agent: String,
        name: String,
    },
}

const EVENT_CHANNEL_CAPACITY: usize = 256;

/// MCP bridge owner.
pub struct McpHandler {
    bridge: RwLock<Arc<McpBridge>>,
    /// Per-fingerprint peer state.
    peers: SyncRwLock<BTreeMap<Fingerprint, PeerEntry>>,
    /// Reverse lookup — (agent, mcp name) → fingerprint of the owning peer.
    by_owner: SyncRwLock<BTreeMap<(String, String), Fingerprint>>,
    events_tx: broadcast::Sender<McpEvent>,
}

impl McpHandler {
    /// Timeout for connecting to a single MCP server.
    const MCP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub fn empty() -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            bridge: RwLock::new(Arc::new(McpBridge::new())),
            peers: SyncRwLock::new(BTreeMap::new()),
            by_owner: SyncRwLock::new(BTreeMap::new()),
            events_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<McpEvent> {
        self.events_tx.subscribe()
    }

    /// Snapshot of every peer's state, keyed by user-facing (agent, name).
    pub fn states(&self) -> BTreeMap<(String, String), McpServerState> {
        let by_owner = self.by_owner.read();
        let peers = self.peers.read();
        by_owner
            .iter()
            .filter_map(|(key, fp)| peers.get(fp).map(|p| (key.clone(), p.state.clone())))
            .collect()
    }

    /// `(peer_id, tool_name)` pairs for every tool exposed by the
    /// agent's declared MCPs. Iteration order matches `mcp_names`, which
    /// matches the agent's declaration order — first-declarer wins on
    /// tool name collisions within an agent. Used by the dispatcher to
    /// route calls to the right peer without exposing tools the agent
    /// didn't ask for.
    pub fn allowed(&self, agent: &str, mcp_names: &[String]) -> Vec<(String, String)> {
        let by_owner = self.by_owner.read();
        let peers = self.peers.read();
        let mut out = Vec::new();
        for name in mcp_names {
            let key = (agent.to_owned(), name.clone());
            if let Some(fp) = by_owner.get(&key)
                && let Some(peer) = peers.get(fp)
            {
                let id = peer_id(*fp);
                for tool_name in &peer.state.tools {
                    out.push((id.clone(), tool_name.clone()));
                }
            }
        }
        out
    }

    /// Get a clone of the current bridge Arc. Tool calls go through this.
    pub async fn bridge(&self) -> Arc<McpBridge> {
        Arc::clone(&*self.bridge.read().await)
    }

    /// Try to get a clone of the current bridge Arc without blocking.
    pub fn try_bridge(&self) -> Option<Arc<McpBridge>> {
        self.bridge.try_read().ok().map(|g| Arc::clone(&*g))
    }

    /// Register `cfg` as belonging to `agent`. If another agent has
    /// already registered an identical config, this is a refcount bump
    /// — no spawn. Otherwise the peer is spawned in the background and
    /// the result reflected via lifecycle events.
    pub async fn register_for_agent(&self, agent: &str, cfg: &McpServerConfig) {
        let fp = fingerprint(cfg);
        let key = (agent.to_owned(), cfg.name.clone());

        // Fast path — fingerprint already tracked.
        let needs_spawn = {
            let mut peers = self.peers.write();
            let mut by_owner = self.by_owner.write();
            // Drop any prior claim by this owner — same key may have
            // pointed at a different fingerprint before update.
            if let Some(old_fp) = by_owner.insert(key.clone(), fp)
                && old_fp != fp
                && let Some(entry) = peers.get_mut(&old_fp)
            {
                entry.refs.remove(&key);
            }
            match peers.get_mut(&fp) {
                Some(entry) => {
                    entry.refs.insert(key.clone());
                    // Replay the terminal status to the new owner so
                    // subscribers get a uniform view of register events.
                    let event = match &entry.state.status {
                        ServerStatus::Connected => Some(McpEvent::Connected {
                            agent: agent.to_owned(),
                            name: cfg.name.clone(),
                            tools: entry.state.tools.clone(),
                        }),
                        ServerStatus::Failed => Some(McpEvent::Failed {
                            agent: agent.to_owned(),
                            name: cfg.name.clone(),
                            error: entry.state.last_error.clone().unwrap_or_default(),
                        }),
                        ServerStatus::Connecting | ServerStatus::Disconnected => None,
                    };
                    if let Some(e) = event {
                        let _ = self.events_tx.send(e);
                    }
                    false
                }
                None => {
                    let mut refs = BTreeSet::new();
                    refs.insert(key.clone());
                    peers.insert(
                        fp,
                        PeerEntry {
                            state: McpServerState::connecting(),
                            refs,
                        },
                    );
                    true
                }
            }
        };

        let _ = self.events_tx.send(McpEvent::Connecting {
            agent: agent.to_owned(),
            name: cfg.name.clone(),
        });

        if !needs_spawn {
            return;
        }

        // Cold path — actually spawn the peer.
        let bridge = self.bridge().await;
        let state = connect_one(&bridge, cfg, fp).await;
        {
            let mut peers = self.peers.write();
            if let Some(entry) = peers.get_mut(&fp) {
                entry.state = state.clone();
            }
        }
        let event = match &state.status {
            ServerStatus::Connected => McpEvent::Connected {
                agent: agent.to_owned(),
                name: cfg.name.clone(),
                tools: state.tools.clone(),
            },
            ServerStatus::Failed => McpEvent::Failed {
                agent: agent.to_owned(),
                name: cfg.name.clone(),
                error: state.last_error.clone().unwrap_or_default(),
            },
            ServerStatus::Connecting | ServerStatus::Disconnected => return,
        };
        let _ = self.events_tx.send(event);
    }

    /// Drop the agent's claim on the named MCP. When the last claim is
    /// released the peer is disconnected and forgotten.
    pub async fn unregister_for_agent(&self, agent: &str, name: &str) {
        let key = (agent.to_owned(), name.to_owned());
        let drop_fp: Option<Fingerprint> = {
            let mut by_owner = self.by_owner.write();
            let Some(fp) = by_owner.remove(&key) else {
                return;
            };
            let mut peers = self.peers.write();
            if let Some(entry) = peers.get_mut(&fp) {
                entry.refs.remove(&key);
                if entry.refs.is_empty() {
                    peers.remove(&fp);
                    Some(fp)
                } else {
                    None
                }
            } else {
                None
            }
        };

        let _ = self.events_tx.send(McpEvent::Disconnected {
            agent: agent.to_owned(),
            name: name.to_owned(),
        });

        if let Some(fp) = drop_fp {
            let bridge = self.bridge().await;
            bridge.remove_server(&peer_id(fp)).await;
        }
    }
}

/// String form of a fingerprint, used as the bridge's peer key. Bridge
/// remains name-keyed; we hand it the fingerprint hex.
pub(crate) fn peer_id(fp: Fingerprint) -> String {
    format!("{:016x}", fp)
}

/// Attempt to connect a single server, applying the global timeout.
async fn connect_one(bridge: &McpBridge, cfg: &McpServerConfig, fp: Fingerprint) -> McpServerState {
    let id = peer_id(fp);
    let fut = async {
        if let Some(url) = &cfg.url {
            tracing::info!(
                server = %cfg.name,
                fingerprint = %id,
                %url,
                "connecting MCP server via HTTP"
            );
            bridge
                .connect_http_named(id.clone(), url, cfg.auth.clone())
                .await
        } else {
            let mut cmd = tokio::process::Command::new(&cfg.command);
            cmd.args(&cfg.args);
            for (k, v) in &cfg.env {
                cmd.env(k, v);
            }
            tracing::info!(
                server = %cfg.name,
                fingerprint = %id,
                command = %cfg.command,
                "connecting MCP server via stdio"
            );
            bridge.connect_stdio_named(id.clone(), cmd).await
        }
    };

    match tokio::time::timeout(McpHandler::MCP_CONNECT_TIMEOUT, fut).await {
        Ok(Ok(tools)) => {
            tracing::info!(
                "connected MCP server '{}' ({}) — {} tool(s)",
                cfg.name,
                id,
                tools.len()
            );
            McpServerState::connected(tools)
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            tracing::warn!("failed to connect MCP server '{}' ({id}): {msg}", cfg.name);
            McpServerState::failed(msg)
        }
        Err(_) => {
            let msg = format!(
                "timed out after {}s",
                McpHandler::MCP_CONNECT_TIMEOUT.as_secs()
            );
            tracing::warn!("MCP server '{}' ({id}) {msg}, skipping", cfg.name);
            McpServerState::failed(msg)
        }
    }
}

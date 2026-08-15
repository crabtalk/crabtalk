//! Crabtalk MCP handler — agent-driven registration with fingerprint-keyed dedup.
//!
//! Agents declare their MCP servers inline (RFC 0193). The handler tracks
//! which agents have declared which configs and dedups identical configs
//! by structural fingerprint — two agents declaring the exact same
//! `(command, args, env, url, auth)` share one peer process.
//!
//! Declaration and process have separate lifetimes. Registering records
//! the config and nothing more; the process starts on the agent's first
//! MCP tool call and the reaper stops it once it goes idle, leaving the
//! declaration behind to start again from. Only the last owner
//! unregistering removes the declaration itself.

use crate::{McpBridge, bridge::CallError};
use parking_lot::RwLock as SyncRwLock;
use std::{
    collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::McpServerConfig;

/// Stable identifier for a peer process — hash of the structural config.
/// Two configs with the same fingerprint produce the same peer; different
/// fingerprints get separate peers.
pub type Fingerprint = u64;

/// Compute the dedup fingerprint for a config. Hashes the fields that
/// affect peer identity: command, args, env, url, and auth. Credentials
/// are identity — one peer sends one `Authorization` header on behalf of
/// every agent sharing it, so agents holding different tokens must not
/// share a peer. `name` and `auto_restart` are not part of the
/// fingerprint — they are presentation-level.
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
    cfg.auth.hash(&mut h);
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

    /// Declared but not running — either never started or reaped. Tools
    /// are cleared because none are reachable until it comes back.
    fn idle() -> Self {
        Self {
            status: ServerStatus::Disconnected,
            tools: Vec::new(),
            last_error: None,
        }
    }
}

/// One peer's tracked state plus the (agent, name) pairs that own it.
///
/// An entry is a *declaration*, not a process. It appears when an agent
/// registers the config and survives idle eviction — only `refs` going
/// empty removes it. Whether a process is currently running behind it is
/// `state.status`.
#[derive(Debug)]
struct PeerEntry {
    state: McpServerState,
    /// Owners — at least one. When this drops to empty the peer is torn down.
    refs: BTreeSet<(String, String)>,
    /// The config this peer was spawned from, env overlay already applied.
    /// Kept so a reconnect can respawn the same process without asking a
    /// caller to reproduce the effective config — the fingerprint is
    /// derived from it, so any drift would strand the peer under an id
    /// that no longer describes it.
    cfg: McpServerConfig,
    /// Last tool call routed here. `None` while no process is running.
    last_used: Option<Instant>,
    /// Held across a connect so concurrent dispatches for the same peer
    /// wait for one spawn instead of racing to start several.
    gate: Arc<Mutex<()>>,
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
    /// How long a peer may sit unused before the reaper stops it. Zero
    /// disables eviction.
    idle_timeout: Duration,
}

impl McpHandler {
    /// Timeout for connecting to a single MCP server.
    const MCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

    /// The reaper wakes this often relative to `idle_timeout`, so a peer
    /// outlives its deadline by at most a quarter of it.
    const REAP_DIVISOR: u32 = 4;

    pub fn empty() -> Self {
        Self::new(Duration::ZERO)
    }

    pub fn new(idle_timeout: Duration) -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            bridge: RwLock::new(Arc::new(McpBridge::new())),
            peers: SyncRwLock::new(BTreeMap::new()),
            by_owner: SyncRwLock::new(BTreeMap::new()),
            events_tx,
            idle_timeout,
        }
    }

    /// Start the idle reaper. Holds a weak reference, so the task ends
    /// when the last real owner drops the handler.
    pub fn spawn_reaper(self: &Arc<Self>) {
        if self.idle_timeout.is_zero() {
            return;
        }
        let tick = self.idle_timeout / Self::REAP_DIVISOR;
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick);
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(handler) = weak.upgrade() else {
                    return;
                };
                handler.reap_idle().await;
            }
        });
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

    /// `(fingerprint, tool_name)` pairs for every tool exposed by the
    /// agent's declared MCPs. Iteration order matches `mcp_names`, which
    /// matches the agent's declaration order — first-declarer wins on
    /// tool name collisions within an agent. Used by the dispatcher to
    /// route calls to the right peer without exposing tools the agent
    /// didn't ask for.
    pub fn allowed(&self, agent: &str, mcp_names: &[String]) -> Vec<(Fingerprint, String)> {
        let by_owner = self.by_owner.read();
        let peers = self.peers.read();
        let mut out = Vec::new();
        for name in mcp_names {
            let key = (agent.to_owned(), name.clone());
            if let Some(fp) = by_owner.get(&key)
                && let Some(peer) = peers.get(fp)
            {
                for tool_name in &peer.state.tools {
                    out.push((*fp, tool_name.clone()));
                }
            }
        }
        out
    }

    /// Call a tool on a peer. Routed through the handler rather than
    /// straight to the bridge so a call that never reached the far side
    /// flips the peer to `Failed` — the bridge owns connections, the
    /// handler owns the state clients observe.
    pub async fn call(
        &self,
        fp: Fingerprint,
        tool_name: &str,
        arguments: &str,
    ) -> Result<String, String> {
        if let Some(entry) = self.peers.write().get_mut(&fp) {
            entry.last_used = Some(Instant::now());
        }
        let bridge = self.bridge().await;
        match bridge.call(&peer_id(fp), tool_name, arguments).await {
            Ok(output) => Ok(output),
            Err(CallError::Rejected(msg)) => Err(msg),
            Err(CallError::Transport(msg)) => {
                self.mark_failed(fp, &msg);
                Err(msg)
            }
        }
    }

    /// Flip a peer to `Failed` and notify every agent that owns it.
    /// Tools are left in place: they describe what the peer exports, not
    /// whether it is reachable, and clearing them would turn a dead
    /// connection into a misleading "tool not available".
    fn mark_failed(&self, fp: Fingerprint, error: &str) {
        let owners: Vec<(String, String)> = {
            let mut peers = self.peers.write();
            let Some(entry) = peers.get_mut(&fp) else {
                return;
            };
            if entry.state.status == ServerStatus::Failed {
                return;
            }
            entry.state.status = ServerStatus::Failed;
            entry.state.last_error = Some(error.to_owned());
            entry.refs.iter().cloned().collect()
        };
        for (agent, name) in owners {
            let _ = self.events_tx.send(McpEvent::Failed {
                agent,
                name,
                error: error.to_owned(),
            });
        }
    }

    /// Get a clone of the current bridge Arc. Tool calls go through this.
    pub async fn bridge(&self) -> Arc<McpBridge> {
        Arc::clone(&*self.bridge.read().await)
    }

    /// Try to get a clone of the current bridge Arc without blocking.
    pub fn try_bridge(&self) -> Option<Arc<McpBridge>> {
        self.bridge.try_read().ok().map(|g| Arc::clone(&*g))
    }

    /// Record `cfg` as belonging to `agent`. No process is started: a
    /// declaration is not a connection, and a daemon holding thousands
    /// of agents would otherwise spawn a child for every MCP any of them
    /// ever mentioned. [`ensure_connected`](Self::ensure_connected)
    /// spawns on first use; the reaper stops it again when it goes idle.
    ///
    /// Identical configs across agents still share one declaration, so
    /// this stays a refcount bump for the second and later owners.
    pub async fn register_for_agent(&self, agent: &str, cfg: &McpServerConfig) {
        let fp = fingerprint(cfg);
        let key = (agent.to_owned(), cfg.name.clone());

        let stale = {
            let mut peers = self.peers.write();
            let mut by_owner = self.by_owner.write();
            // Drop any prior claim by this owner — same key may have
            // pointed at a different fingerprint before update.
            let mut stale = None;
            if let Some(old_fp) = by_owner.insert(key.clone(), fp)
                && old_fp != fp
                && let Some(entry) = peers.get_mut(&old_fp)
            {
                entry.refs.remove(&key);
                if entry.refs.is_empty() {
                    peers.remove(&old_fp);
                    stale = Some(old_fp);
                }
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
                }
                None => {
                    let mut refs = BTreeSet::new();
                    refs.insert(key.clone());
                    peers.insert(
                        fp,
                        PeerEntry {
                            state: McpServerState::idle(),
                            refs,
                            cfg: cfg.clone(),
                            last_used: None,
                            gate: Arc::new(Mutex::new(())),
                        },
                    );
                }
            }
            stale
        };

        // The claim we just moved may have been the last one on its old
        // peer — tear it down. Nothing replaces it until first use.
        if let Some(old_fp) = stale {
            self.bridge().await.remove_server(&peer_id(old_fp)).await;
        }
    }

    /// Bring up every peer the agent declared that isn't already running.
    /// Called before dispatch, so a tool call is what pays for the spawn.
    ///
    /// Connects are concurrent across MCPs but serialised per peer — two
    /// agents reaching a shared peer at once wait on one spawn rather
    /// than starting two processes under the same id.
    pub async fn ensure_connected(&self, agent: &str, mcp_names: &[String]) {
        let pending: Vec<Fingerprint> = {
            let by_owner = self.by_owner.read();
            let peers = self.peers.read();
            mcp_names
                .iter()
                .filter_map(|name| by_owner.get(&(agent.to_owned(), name.clone())))
                // `Failed` is deliberately not retried here: a peer whose
                // command is wrong would burn a connect timeout on every
                // turn. It becomes eligible again once the reaper ages it
                // back to `Disconnected`, which bounds the retry rate to
                // the idle timeout without any backoff machinery. To retry
                // sooner, ask for it — that is what `ReconnectMcp` is.
                // `Connecting` is included so a caller arriving mid-spawn
                // waits on the gate rather than seeing an empty tool list.
                .filter(|fp| {
                    peers.get(fp).is_some_and(|e| {
                        matches!(
                            e.state.status,
                            ServerStatus::Disconnected | ServerStatus::Connecting
                        )
                    })
                })
                .copied()
                .collect()
        };
        if pending.is_empty() {
            return;
        }
        futures_util::future::join_all(pending.into_iter().map(|fp| self.connect_peer(fp))).await;
    }

    /// Spawn the peer behind `fp` unless it is already up.
    async fn connect_peer(&self, fp: Fingerprint) {
        let Some((gate, cfg)) = ({
            let peers = self.peers.read();
            peers.get(&fp).map(|e| (e.gate.clone(), e.cfg.clone()))
        }) else {
            return;
        };
        let _held = gate.lock().await;

        // Re-check under the gate: whoever held it before us may have
        // already settled this peer, either way.
        let owners: Vec<(String, String)> = {
            let mut peers = self.peers.write();
            let Some(entry) = peers.get_mut(&fp) else {
                return;
            };
            if matches!(
                entry.state.status,
                ServerStatus::Connected | ServerStatus::Failed
            ) {
                return;
            }
            entry.state = McpServerState::connecting();
            entry.refs.iter().cloned().collect()
        };
        self.broadcast(&owners, |agent, name| McpEvent::Connecting { agent, name });

        let bridge = self.bridge().await;
        let state = connect_one(&bridge, &cfg, fp).await;
        {
            let mut peers = self.peers.write();
            if let Some(entry) = peers.get_mut(&fp) {
                entry.state = state.clone();
                entry.last_used = Some(Instant::now());
            }
        }
        self.announce(&owners, &state);
    }

    /// Stop peers that have gone quiet. The declaration stays, so the
    /// next call reconnects — this reclaims processes, not config.
    async fn reap_idle(&self) {
        let now = Instant::now();
        let expired: Vec<(Fingerprint, Vec<(String, String)>)> = {
            let mut peers = self.peers.write();
            peers
                .iter_mut()
                // `Failed` peers count too: the transport broke, but the
                // process is still running and still costs a slot.
                .filter(|(_, e)| {
                    matches!(
                        e.state.status,
                        ServerStatus::Connected | ServerStatus::Failed
                    )
                })
                .filter(|(_, e)| {
                    e.last_used
                        .is_some_and(|t| now.duration_since(t) >= self.idle_timeout)
                })
                .map(|(fp, e)| {
                    // Field-wise rather than `idle()` so a failure reason
                    // survives into the listing after the peer is gone.
                    e.state.status = ServerStatus::Disconnected;
                    e.state.tools.clear();
                    e.last_used = None;
                    (*fp, e.refs.iter().cloned().collect())
                })
                .collect()
        };
        if expired.is_empty() {
            return;
        }
        let bridge = self.bridge().await;
        for (fp, owners) in expired {
            bridge.remove_server(&peer_id(fp)).await;
            tracing::info!(fingerprint = %peer_id(fp), "stopped idle MCP peer");
            self.broadcast(&owners, |agent, name| McpEvent::Disconnected {
                agent,
                name,
            });
        }
    }

    /// Emit one event per owner of a peer.
    fn broadcast(&self, owners: &[(String, String)], make: impl Fn(String, String) -> McpEvent) {
        for (agent, name) in owners {
            let _ = self.events_tx.send(make(agent.clone(), name.clone()));
        }
    }

    /// Tell every owner how a connect attempt ended.
    fn announce(&self, owners: &[(String, String)], state: &McpServerState) {
        match state.status {
            ServerStatus::Connected => self.broadcast(owners, |agent, name| McpEvent::Connected {
                agent,
                name,
                tools: state.tools.clone(),
            }),
            ServerStatus::Failed => self.broadcast(owners, |agent, name| McpEvent::Failed {
                agent,
                name,
                error: state.last_error.clone().unwrap_or_default(),
            }),
            ServerStatus::Connecting | ServerStatus::Disconnected => {}
        }
    }

    /// Tear down the peer backing `(agent, name)` and connect it again,
    /// keeping every owner's claim.
    ///
    /// Reconnecting is per-peer, not per-claim: a process shared by
    /// several agents comes back once for all of them, and they all see
    /// the lifecycle events. The config comes from the peer itself, so a
    /// reconnect always respawns what was actually running — changing the
    /// config is `register_for_agent`'s job, and it mints a new peer.
    pub async fn reconnect_for_agent(&self, agent: &str, name: &str) -> Result<(), String> {
        let key = (agent.to_owned(), name.to_owned());
        let Some(fp) = self.by_owner.read().get(&key).copied() else {
            return Err(format!(
                "mcp '{name}' is not registered for agent '{agent}'"
            ));
        };
        // Drop the process first so `connect_peer` sees a peer that needs
        // starting — otherwise it would take the already-connected path
        // and reconnect would be a no-op.
        {
            let mut peers = self.peers.write();
            let Some(entry) = peers.get_mut(&fp) else {
                return Err(format!("mcp '{name}' is not tracked"));
            };
            entry.state = McpServerState::idle();
        }
        self.bridge().await.remove_server(&peer_id(fp)).await;
        self.connect_peer(fp).await;
        Ok(())
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

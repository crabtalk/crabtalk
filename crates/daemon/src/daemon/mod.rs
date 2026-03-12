//! Daemon — the core struct composing runtime, transports, and lifecycle.
//!
//! [`Daemon`] owns the runtime and shared state. [`DaemonHandle`] owns the
//! spawned tasks and provides graceful shutdown. Transport setup is
//! decomposed into private helpers called from [`Daemon::start`].

use crate::{
    DaemonConfig,
    daemon::event::{DaemonEvent, DaemonEventSender},
    hook::DaemonHook,
};
use ::socket::server::accept_loop;
use anyhow::Result;
use model::ProviderManager;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};
use wcore::AgentConfig;
use wcore::Runtime;
use wcore::protocol::message::client::ClientMessage;

pub(crate) mod builder;
pub mod event;
mod protocol;

/// Shared daemon state — holds the runtime. Cheap to clone (`Arc`-backed).
///
/// The runtime is stored behind `Arc<RwLock<Arc<Runtime>>>` so that
/// [`Daemon::reload`] can swap it atomically while in-flight requests that
/// already cloned the inner `Arc` complete normally.
#[derive(Clone)]
pub struct Daemon {
    /// The walrus runtime, swappable via [`Daemon::reload`].
    pub runtime: Arc<RwLock<Arc<Runtime<ProviderManager, DaemonHook>>>>,
    /// Config directory — stored so [`Daemon::reload`] can re-read config from disk.
    pub(crate) config_dir: PathBuf,
    /// Sender for the daemon event loop — cloned into `Runtime` as `ToolSender`
    /// so agents can dispatch tool calls. Stored here so [`Daemon::reload`] can
    /// pass a fresh clone into the rebuilt runtime.
    pub(crate) event_tx: DaemonEventSender,
    /// Per-agent configurations (name → config).
    pub(crate) agents_config: BTreeMap<String, AgentConfig>,
}

impl Daemon {
    /// Load config, build runtime, and start the event loop.
    ///
    /// Returns a [`DaemonHandle`] with the event sender exposed. The caller
    /// spawns transports (socket, channels) using the handle's `event_tx`
    /// and `shutdown_tx`, then integrates its own channels by cloning
    /// `event_tx` and sending [`DaemonEvent::Message`] variants.
    pub async fn start(config_dir: &Path) -> Result<DaemonHandle> {
        let config_path = config_dir.join("walrus.toml");
        let config = DaemonConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (event_tx, event_rx) = mpsc::unbounded_channel::<DaemonEvent>();
        let daemon = Daemon::build(&config, config_dir, event_tx.clone()).await?;

        // Broadcast shutdown — all subsystems subscribe.
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_event_tx = event_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
            let _ = shutdown_event_tx.send(DaemonEvent::Shutdown);
        });

        // Per-agent heartbeat timers — only agents with interval > 0 run.
        for (name, agent) in &config.agents {
            if agent.heartbeat.interval == 0 {
                continue;
            }
            let agent_name = compact_str::CompactString::from(name.as_str());
            let heartbeat_tx = event_tx.clone();
            let mut heartbeat_shutdown = shutdown_tx.subscribe();
            let interval_secs = agent.heartbeat.interval * 60;
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                tick.tick().await; // skip the immediate first tick
                loop {
                    tokio::select! {
                        _ = tick.tick() => {
                            let event = DaemonEvent::Heartbeat {
                                agent: agent_name.clone(),
                            };
                            if heartbeat_tx.send(event).is_err() {
                                break;
                            }
                        }
                        _ = heartbeat_shutdown.recv() => break,
                    }
                }
            });
            tracing::info!(
                "heartbeat timer started for '{}' (interval: {}m)",
                name,
                agent.heartbeat.interval,
            );
        }

        let d = daemon.clone();
        let event_loop_join = tokio::spawn(async move {
            d.handle_events(event_rx).await;
        });

        Ok(DaemonHandle {
            config,
            event_tx,
            shutdown_tx,
            daemon,
            event_loop_join: Some(event_loop_join),
        })
    }
}

/// Handle returned by [`Daemon::start`] — holds the event sender and shutdown trigger.
///
/// The caller spawns transports (socket, channels) using [`setup_socket`] and
/// [`setup_channels`], passing clones of `event_tx` and `shutdown_tx`.
pub struct DaemonHandle {
    /// The loaded daemon configuration.
    pub config: DaemonConfig,
    /// Sender for injecting events into the daemon event loop.
    /// Clone this and pass to transport setup functions.
    pub event_tx: DaemonEventSender,
    /// Broadcast shutdown — call `.subscribe()` for transport shutdown,
    /// or use [`DaemonHandle::shutdown`] to trigger.
    pub shutdown_tx: broadcast::Sender<()>,
    daemon: Daemon,
    event_loop_join: Option<tokio::task::JoinHandle<()>>,
}

impl DaemonHandle {
    /// Wait until the active model provider is ready.
    ///
    /// Call this after socket setup so transports are available while the
    /// model loads. No-op for remote providers.
    pub async fn wait_until_ready(&self) -> Result<()> {
        let rt = self.daemon.runtime.read().await;
        rt.model.wait_until_ready().await
    }

    /// Trigger graceful shutdown and wait for the event loop to stop.
    ///
    /// Transport tasks (socket, channels) are the caller's responsibility.
    pub async fn shutdown(mut self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        if let Some(join) = self.event_loop_join.take() {
            join.await?;
        }
        Ok(())
    }
}

// ── Transport setup helpers ──────────────────────────────────────────

/// Bind the Unix domain socket and spawn the accept loop.
pub fn setup_socket(
    shutdown_tx: &broadcast::Sender<()>,
    event_tx: &DaemonEventSender,
) -> Result<(&'static Path, tokio::task::JoinHandle<()>)> {
    let resolved_path: &'static Path = &wcore::paths::SOCKET_PATH;
    if let Some(parent) = resolved_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if resolved_path.exists() {
        std::fs::remove_file(resolved_path)?;
    }

    let listener = tokio::net::UnixListener::bind(resolved_path)?;
    tracing::info!("daemon listening on {}", resolved_path.display());

    let socket_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let socket_tx = event_tx.clone();
    let join = tokio::spawn(accept_loop(
        listener,
        move |msg, reply| {
            let _ = socket_tx.send(DaemonEvent::Message { msg, reply });
        },
        socket_shutdown,
    ));

    Ok((resolved_path, join))
}

/// Spawn channel transports.
pub async fn setup_channels(config: &DaemonConfig, event_tx: &DaemonEventSender) {
    let tx = event_tx.clone();
    let on_message = Arc::new(move |msg: ClientMessage| {
        let tx = tx.clone();
        async move {
            let (reply_tx, reply_rx) = mpsc::unbounded_channel();
            let _ = tx.send(DaemonEvent::Message {
                msg,
                reply: reply_tx,
            });
            reply_rx
        }
    });

    // Use the first configured agent name as the default, falling back to "assistant".
    let agents_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    let default_agent = crate::config::load_agents_dir(&agents_dir)
        .ok()
        .and_then(|agents| agents.into_iter().next())
        .map(|(stem, _)| compact_str::CompactString::from(stem))
        .unwrap_or_else(|| compact_str::CompactString::from("assistant"));
    channel::spawn_channels(&config.channel, default_agent, on_message).await;
}

/// Bind a TCP listener and spawn the accept loop.
pub fn setup_tcp(
    addr: std::net::SocketAddr,
    shutdown_tx: &broadcast::Sender<()>,
    event_tx: &DaemonEventSender,
) -> Result<tokio::task::JoinHandle<()>> {
    let listener = tokio::net::TcpListener::from_std(std::net::TcpListener::bind(addr)?)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let tcp_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let tcp_tx = event_tx.clone();
    let join = tokio::spawn(tcp::server::accept_loop(
        listener,
        move |msg, reply| {
            let _ = tcp_tx.send(DaemonEvent::Message { msg, reply });
        },
        tcp_shutdown,
    ));

    Ok(join)
}

/// Bridge a broadcast receiver into a oneshot receiver.
pub fn bridge_shutdown(mut rx: broadcast::Receiver<()>) -> oneshot::Receiver<()> {
    let (otx, orx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = rx.recv().await;
        let _ = otx.send(());
    });
    orx
}

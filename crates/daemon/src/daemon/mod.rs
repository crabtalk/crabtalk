//! Daemon — the core struct composing runtime, transports, and lifecycle.
//!
//! [`Daemon`] owns the runtime and shared state. [`DaemonHandle`] owns the
//! spawned tasks and provides graceful shutdown. Transport setup is
//! decomposed into private helpers called from [`Daemon::start`].

use crate::{
    DaemonConfig,
    daemon::event::{DaemonEvent, DaemonEventSender},
    hook::DaemonHook,
    service::ServiceManager,
};
use ::socket::server::accept_loop;
use anyhow::Result;
use model::ProviderRegistry;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};
use wcore::AgentConfig;
use wcore::Runtime;

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
    pub runtime: Arc<RwLock<Arc<Runtime<ProviderRegistry, DaemonHook>>>>,
    /// Config directory — stored so [`Daemon::reload`] can re-read config from disk.
    pub(crate) config_dir: PathBuf,
    /// Sender for the daemon event loop — cloned into `Runtime` as `ToolSender`
    /// so agents can dispatch tool calls. Stored here so [`Daemon::reload`] can
    /// pass a fresh clone into the rebuilt runtime.
    pub(crate) event_tx: DaemonEventSender,
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
        let (daemon, service_manager) =
            Daemon::build(&config, config_dir, event_tx.clone()).await?;

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
            service_manager,
        })
    }
}

/// Handle returned by [`Daemon::start`] — holds the event sender and shutdown trigger.
///
/// The caller spawns transports (socket, TCP) using [`setup_socket`] and
/// [`setup_tcp`], passing clones of `event_tx` and `shutdown_tx`.
pub struct DaemonHandle {
    /// The loaded daemon configuration.
    pub config: DaemonConfig,
    /// Sender for injecting events into the daemon event loop.
    /// Clone this and pass to transport setup functions.
    pub event_tx: DaemonEventSender,
    /// Broadcast shutdown — call `.subscribe()` for transport shutdown,
    /// or use [`DaemonHandle::shutdown`] to trigger.
    pub shutdown_tx: broadcast::Sender<()>,
    #[allow(unused)]
    daemon: Daemon,
    event_loop_join: Option<tokio::task::JoinHandle<()>>,
    /// Managed child services — shutdown on daemon stop.
    service_manager: Option<ServiceManager>,
}

impl DaemonHandle {
    /// Wait until the active model provider is ready.
    ///
    /// No-op for remote providers. Kept for API compatibility.
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    /// Trigger graceful shutdown and wait for the event loop to stop.
    ///
    /// Transport tasks (socket, channels) are the caller's responsibility.
    pub async fn shutdown(mut self) -> Result<()> {
        // Shutdown managed services before the event loop.
        if let Some(ref mut sm) = self.service_manager {
            sm.shutdown_all().await;
        }
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

/// Bind a TCP listener and spawn the accept loop.
///
/// Tries the default port (6688), falls back to an OS-assigned port.
/// Returns the join handle and the actual port bound.
pub fn setup_tcp(
    shutdown_tx: &broadcast::Sender<()>,
    event_tx: &DaemonEventSender,
) -> Result<(tokio::task::JoinHandle<()>, u16)> {
    let (std_listener, addr) = tcp::server::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
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

    Ok((join, addr.port()))
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

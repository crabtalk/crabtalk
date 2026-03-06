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
use compact_str::CompactString;
use model::ProviderManager;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};
use wcore::Runtime;

pub(crate) mod builder;
pub(crate) mod event;
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
}

impl Daemon {
    /// Load config, build runtime, bind the Unix domain socket, and start serving.
    ///
    /// Returns a [`DaemonHandle`] with the socket path and a shutdown trigger.
    pub async fn start(config_dir: &Path) -> Result<DaemonHandle> {
        let config_path = config_dir.join("walrus.toml");
        let config = DaemonConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());
        Self::start_with_config(&config, config_dir).await
    }

    /// Start with an already-loaded config. Useful when the caller resolves
    /// config separately (e.g. CLI with scaffold logic).
    pub async fn start_with_config(
        config: &DaemonConfig,
        config_dir: &Path,
    ) -> Result<DaemonHandle> {
        let (event_tx, event_rx) = mpsc::unbounded_channel::<DaemonEvent>();
        let daemon = Daemon::build(config, config_dir, event_tx.clone()).await?;

        // Broadcast shutdown — all subsystems subscribe.
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_event_tx = event_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
            let _ = shutdown_event_tx.send(DaemonEvent::Shutdown);
        });

        let (socket_path, socket_join) = setup_socket(&shutdown_tx, &event_tx)?;
        setup_channels(config, &event_tx).await;

        let d = daemon.clone();
        let event_loop_join = tokio::spawn(async move {
            d.handle_events(event_rx).await;
        });

        Ok(DaemonHandle {
            socket_path,
            shutdown_tx: Some(shutdown_tx),
            socket_join: Some(socket_join),
            event_loop_join: Some(event_loop_join),
        })
    }
}

/// Handle returned by [`Daemon::start`] — holds the socket path and shutdown trigger.
pub struct DaemonHandle {
    /// The Unix domain socket path the daemon is listening on.
    pub socket_path: &'static Path,
    shutdown_tx: Option<broadcast::Sender<()>>,
    socket_join: Option<tokio::task::JoinHandle<()>>,
    event_loop_join: Option<tokio::task::JoinHandle<()>>,
}

impl DaemonHandle {
    /// Trigger graceful shutdown and wait for all subsystems to stop.
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.socket_join.take() {
            join.await?;
        }
        if let Some(join) = self.event_loop_join.take() {
            join.await?;
        }
        let _ = std::fs::remove_file(self.socket_path);
        Ok(())
    }
}

// ── Transport setup helpers ──────────────────────────────────────────

/// Bind the Unix domain socket and spawn the accept loop.
fn setup_socket(
    shutdown_tx: &broadcast::Sender<()>,
    event_tx: &DaemonEventSender,
) -> Result<(&'static Path, tokio::task::JoinHandle<()>)> {
    let resolved_path: &'static Path = &crate::config::SOCKET_PATH;
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
            let _ = socket_tx.send(DaemonEvent::Socket { msg, reply });
        },
        socket_shutdown,
    ));

    Ok((resolved_path, join))
}

/// Build the channel router and spawn channel transports.
async fn setup_channels(config: &DaemonConfig, event_tx: &DaemonEventSender) {
    let channels = config.channels.values().cloned().collect::<Vec<_>>();
    let router = channel::build_router(&channels);
    let router = Arc::new(router);
    let channel_tx = event_tx.clone();
    let on_message = Arc::new(move |agent: CompactString, content: String| {
        let tx = channel_tx.clone();
        async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            let event = DaemonEvent::Channel {
                agent,
                content,
                reply: reply_tx,
            };
            if tx.send(event).is_err() {
                return Err("event loop closed".to_owned());
            }
            reply_rx
                .await
                .unwrap_or(Err("event loop dropped".to_owned()))
        }
    });
    channel::spawn_channels(&channels, router, on_message).await;
}

/// Bridge a broadcast receiver into a oneshot receiver.
fn bridge_shutdown(mut rx: broadcast::Receiver<()>) -> oneshot::Receiver<()> {
    let (otx, orx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = rx.recv().await;
        let _ = otx.send(());
    });
    orx
}

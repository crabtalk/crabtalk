//! Daemon — the core struct composing runtime, transports, and lifecycle.
//!
//! [`Daemon`] owns the runtime and shared state. [`DaemonHandle`] owns the
//! spawned tasks and provides graceful shutdown. Transport setup is
//! decomposed into private helpers called from [`Daemon::start`].

use crate::{
    DaemonConfig,
    cron::CronStore,
    daemon::{
        builder::{BuildProvider, DefaultProvider, build_default_provider},
        event::{DaemonEvent, DaemonEventSender},
    },
    event_bus::EventBus,
    hook::host::DaemonHost,
};
use anyhow::Result;
use crabllm_core::Provider;
use runtime::{Env, host::Host};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use wcore::{Runtime, model::Model};

pub(crate) mod builder;
pub mod event;
mod protocol;

/// Shared daemon state — holds the runtime. Cheap to clone (`Arc`-backed).
///
/// Generic over the provider type [`P`] (`crabllm_core::Provider`) and the
/// [`Host`] type so downstream binaries can inject custom provider
/// implementations (e.g. local MLX inference) and server-specific tool
/// dispatch. The default concrete P is [`DefaultProvider`] —
/// `Retrying<ProviderRegistry<RemoteProvider>>` — used by `Daemon::start`.
///
/// The runtime is stored behind `Arc<RwLock<Arc<Runtime>>>` so that
/// [`Daemon::reload`] can swap it atomically while in-flight requests that
/// already cloned the inner `Arc` complete normally.
pub struct Daemon<P: Provider + 'static = DefaultProvider, B: Host + 'static = DaemonHost> {
    /// The crabtalk runtime, swappable via [`Daemon::reload`].
    #[allow(clippy::type_complexity)]
    pub runtime: Arc<RwLock<Arc<Runtime<P, Env<B>>>>>,
    /// Config directory — stored so [`Daemon::reload`] can re-read config from disk.
    pub(crate) config_dir: PathBuf,
    /// Sender for the daemon event loop — cloned into `Runtime` as `ToolSender`
    /// so agents can dispatch tool calls. Stored here so [`Daemon::reload`] can
    /// pass a fresh clone into the rebuilt runtime.
    pub(crate) event_tx: DaemonEventSender,
    /// When the daemon was started (for uptime calculation).
    pub(crate) started_at: std::time::Instant,
    /// Daemon-level cron scheduler. Survives runtime reloads.
    pub(crate) crons: Arc<Mutex<CronStore>>,
    /// Event bus — subscription-based routing. Survives runtime reloads.
    pub(crate) events: Arc<Mutex<EventBus>>,
    /// Closure that builds `Model<P>` from config — called on initial
    /// construction and on every `reload()`.
    pub(crate) build_provider: BuildProvider<P>,
}

impl<P: Provider + 'static, B: Host + 'static> Clone for Daemon<P, B> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            config_dir: self.config_dir.clone(),
            event_tx: self.event_tx.clone(),
            started_at: self.started_at,
            crons: self.crons.clone(),
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
        }
    }
}

impl Daemon<DefaultProvider, DaemonHost> {
    /// Load config, build runtime with the default provider (a retrying
    /// `ProviderRegistry<RemoteProvider>`) and the default [`DaemonHost`],
    /// and start the event loop. This is the entry point the crabtalk CLI
    /// binary uses.
    ///
    /// Library consumers with custom provider types (e.g. the Apple app
    /// injecting MLX) should call [`Daemon::start_with`] directly with
    /// their own provider-builder and backend-builder closures.
    pub async fn start(config_dir: &Path) -> Result<DaemonHandle<DefaultProvider, DaemonHost>> {
        Self::start_with(
            config_dir,
            |config: &DaemonConfig| build_default_provider(config),
            |event_tx| {
                let (events_tx, _) = broadcast::channel(256);
                DaemonHost {
                    event_tx,
                    pending_asks: Arc::new(Mutex::new(std::collections::HashMap::new())),
                    conversation_cwds: Arc::new(Mutex::new(std::collections::HashMap::new())),
                    events_tx,
                }
            },
        )
        .await
    }
}

impl<P: Provider + 'static, B: Host + 'static> Daemon<P, B> {
    /// Load config, build runtime with the given provider-builder and
    /// backend-builder, and start the event loop.
    ///
    /// - `build_provider` receives the loaded [`DaemonConfig`] and returns
    ///   the [`Model<P>`] the runtime will dispatch through. Called once
    ///   here and once on every subsequent [`Daemon::reload`].
    /// - `build_backend` receives the [`DaemonEventSender`] so the backend
    ///   can inject events (e.g. for delegate dispatch).
    ///
    /// The provider-builder is stored on `Daemon` as an `Arc<dyn Fn>` so
    /// reload can re-run it with fresh config — that's why it needs
    /// `Fn + Send + Sync + 'static` rather than `FnOnce`.
    pub async fn start_with<BP, BB>(
        config_dir: &Path,
        build_provider: BP,
        build_backend: BB,
    ) -> Result<DaemonHandle<P, B>>
    where
        BP: Fn(&DaemonConfig) -> Result<Model<P>> + Send + Sync + 'static,
        BB: FnOnce(DaemonEventSender) -> B,
    {
        let config_path = config_dir.join(wcore::paths::CONFIG_FILE);
        let config = DaemonConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (event_tx, event_rx) = mpsc::unbounded_channel::<DaemonEvent>();

        // Broadcast shutdown — all subsystems subscribe.
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_event_tx = event_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
            let _ = shutdown_event_tx.send(DaemonEvent::Shutdown);
        });

        let backend = build_backend(event_tx.clone());
        let build_provider: BuildProvider<P> = Arc::new(build_provider);
        let daemon = Daemon::build(
            &config,
            config_dir,
            event_tx.clone(),
            shutdown_tx.clone(),
            backend,
            build_provider,
        )
        .await?;

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
/// The caller spawns transports (socket, TCP) using [`setup_socket`] and
/// [`setup_tcp`], passing clones of `event_tx` and `shutdown_tx`.
pub struct DaemonHandle<P: Provider + 'static = DefaultProvider, B: Host + 'static = DaemonHost> {
    /// The loaded daemon configuration.
    pub config: DaemonConfig,
    /// Sender for injecting events into the daemon event loop.
    /// Clone this and pass to transport setup functions.
    pub event_tx: DaemonEventSender,
    /// Broadcast shutdown — call `.subscribe()` for transport shutdown,
    /// or use [`DaemonHandle::shutdown`] to trigger.
    pub shutdown_tx: broadcast::Sender<()>,
    /// The shared daemon state — exposed for backend/product servers that
    /// layer additional APIs on top.
    pub daemon: Daemon<P, B>,
    event_loop_join: Option<tokio::task::JoinHandle<()>>,
}

impl<P: Provider + 'static, B: Host + 'static> DaemonHandle<P, B> {
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
        let _ = self.shutdown_tx.send(());
        if let Some(join) = self.event_loop_join.take() {
            join.await?;
        }
        Ok(())
    }
}

// ── Transport setup helpers ──────────────────────────────────────────

/// Bind the Unix domain socket and spawn the accept loop.
#[cfg(unix)]
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
    let join = tokio::spawn(transport::uds::accept_loop(
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
    let (std_listener, addr) = transport::tcp::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let tcp_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let tcp_tx = event_tx.clone();
    let join = tokio::spawn(transport::tcp::accept_loop(
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

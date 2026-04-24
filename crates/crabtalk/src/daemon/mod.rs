//! Daemon — the core struct composing runtime, transports, and lifecycle.

use crate::{DaemonConfig, hooks, storage::FsStorage};
use anyhow::Result;
use crabllm_core::Provider;
use futures_util::{StreamExt, pin_mut};
use runtime::Runtime;
use std::collections::HashMap;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use wcore::protocol::{api::Server, message::ClientMessage};
use {
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    event::EventBus,
    host::DaemonEnv,
};

/// Per-conversation working directory overrides.
pub type ConversationCwds = Arc<Mutex<HashMap<u64, PathBuf>>>;

/// Pending ask_user oneshots (shared with AskUserHook and protocol layer).
pub type PendingAsks = Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>;

pub mod builder;
pub mod event;
pub mod hook;
pub mod host;

/// Config binding for a node.
pub struct DaemonCfg<P: Provider + 'static = DefaultProvider> {
    _marker: std::marker::PhantomData<P>,
}

impl<P: Provider + 'static> runtime::Config for DaemonCfg<P> {
    type Storage = FsStorage;
    type Provider = P;
    type Env = DaemonEnv;
}

/// Shared runtime handle.
pub type SharedRuntime<P> = Arc<RwLock<Arc<Runtime<DaemonCfg<P>>>>>;

/// Shared daemon state.
pub struct Daemon<P: Provider + 'static = DefaultProvider> {
    pub runtime: SharedRuntime<P>,
    /// Composite hook owning all sub-hooks and shared state.
    pub hook: Arc<hook::DaemonHook>,
    pub(crate) config_dir: PathBuf,
    pub(crate) started_at: std::time::Instant,
    pub(crate) events: Arc<parking_lot::Mutex<EventBus>>,
    pub(crate) build_provider: BuildProvider<P>,
    pub(crate) mcp: Arc<mcp::McpHandler>,
    /// OS tools hook — owns conversation CWDs and bash policy.
    pub(crate) os_hook: Arc<hooks::os::OsHook>,
    /// Ask-user hook — owns pending ask oneshots.
    pub(crate) ask_hook: Arc<hooks::ask_user::AskUserHook>,
}

impl<P: Provider + 'static> Clone for Daemon<P> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            hook: self.hook.clone(),
            config_dir: self.config_dir.clone(),
            started_at: self.started_at,
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
            mcp: self.mcp.clone(),
            os_hook: self.os_hook.clone(),
            ask_hook: self.ask_hook.clone(),
        }
    }
}

impl Daemon<DefaultProvider> {
    pub async fn start(config_dir: &Path) -> Result<DaemonHandle<DefaultProvider>> {
        let config_path = config_dir.join(wcore::paths::CONFIG_FILE);
        let config = DaemonConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let build_provider: BuildProvider<DefaultProvider> =
            Arc::new(|config: &DaemonConfig, models: &[String]| {
                build_default_provider(config, models)
            });

        let daemon = Daemon::build(&config, config_dir, build_provider).await?;

        Ok(DaemonHandle {
            config,
            shutdown_tx,
            daemon,
        })
    }
}

pub struct DaemonHandle<P: Provider + 'static = DefaultProvider> {
    pub config: DaemonConfig,
    pub shutdown_tx: broadcast::Sender<()>,
    pub daemon: Daemon<P>,
}

impl<P: Provider + 'static> DaemonHandle<P> {
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }
}

// ── Transport setup helpers ──────────────────────────────────────────

fn dispatch_callback<P: Provider + 'static>(
    daemon: Daemon<P>,
) -> impl Fn(ClientMessage, mpsc::Sender<wcore::protocol::message::ServerMessage>) + Clone + Send + 'static
{
    move |msg, reply| {
        let daemon = daemon.clone();
        tokio::spawn(async move {
            let stream = daemon.dispatch(msg);
            pin_mut!(stream);
            while let Some(server_msg) = stream.next().await {
                if reply.send(server_msg).await.is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(unix)]
pub fn setup_socket<P: Provider + 'static>(
    daemon: Daemon<P>,
    shutdown_tx: &broadcast::Sender<()>,
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
    let join = tokio::spawn(transport::uds::accept_loop(
        listener,
        dispatch_callback(daemon),
        socket_shutdown,
    ));

    Ok((resolved_path, join))
}

pub fn setup_tcp<P: Provider + 'static>(
    daemon: Daemon<P>,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(tokio::task::JoinHandle<()>, u16)> {
    let (std_listener, addr) = transport::tcp::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let tcp_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let join = tokio::spawn(transport::tcp::accept_loop(
        listener,
        dispatch_callback(daemon),
        tcp_shutdown,
    ));

    Ok((join, addr.port()))
}

pub fn bridge_shutdown(mut rx: broadcast::Receiver<()>) -> oneshot::Receiver<()> {
    let (otx, orx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = rx.recv().await;
        let _ = otx.send(());
    });
    orx
}

//! Node — the core struct composing runtime, transports, and lifecycle.

use crate::{NodeConfig, storage::FsStorage};
use anyhow::Result;
use crabllm_core::Provider;
use futures_util::{StreamExt, pin_mut};
use runtime::{Runtime, host::Host};
use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use wcore::{
    model::Model,
    protocol::{api::Server, message::ClientMessage},
};
use {
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    cron::CronStore,
    event::EventBus,
    host::NodeHost,
};

pub mod builder;
pub mod cron;
pub mod event;
pub mod host;

/// Config binding for a node: ties Provider + Host to FsStorage + Env.
pub struct NodeCfg<P: Provider + 'static = DefaultProvider, B: Host + 'static = NodeHost> {
    _marker: PhantomData<(P, B)>,
}

impl<P: Provider + 'static, B: Host + 'static> runtime::Config for NodeCfg<P, B> {
    type Storage = FsStorage;
    type Provider = P;
    type Host = B;
}

/// Shared runtime handle — `Arc<RwLock<Arc<...>>>` so reload can swap
/// the inner `Arc` without disrupting in-flight requests.
pub type SharedRuntime<P, B> = Arc<RwLock<Arc<Runtime<NodeCfg<P, B>>>>>;

/// Shared daemon state.
pub struct Node<P: Provider + 'static = DefaultProvider, B: Host + 'static = NodeHost> {
    pub runtime: SharedRuntime<P, B>,
    pub(crate) config_dir: PathBuf,
    pub(crate) started_at: std::time::Instant,
    pub(crate) crons: Arc<Mutex<CronStore<P, B>>>,
    pub(crate) events: Arc<std::sync::Mutex<EventBus>>,
    pub(crate) build_provider: BuildProvider<P>,
    pub(crate) mcp: Arc<crate::mcp::McpHandler>,
}

impl<P: Provider + 'static, B: Host + 'static> Clone for Node<P, B> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            config_dir: self.config_dir.clone(),
            started_at: self.started_at,
            crons: self.crons.clone(),
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
            mcp: self.mcp.clone(),
        }
    }
}

impl Node<DefaultProvider, NodeHost> {
    pub async fn start(config_dir: &Path) -> Result<NodeHandle<DefaultProvider, NodeHost>> {
        Self::start_with(
            config_dir,
            |config: &NodeConfig| build_default_provider(config),
            || {
                let (events_tx, _) = broadcast::channel(256);
                NodeHost { events_tx }
            },
        )
        .await
    }
}

impl<P: Provider + 'static, B: Host + 'static> Node<P, B> {
    pub async fn start_with<BP, BB>(
        config_dir: &Path,
        build_provider: BP,
        build_backend: BB,
    ) -> Result<NodeHandle<P, B>>
    where
        BP: Fn(&NodeConfig) -> Result<Model<P>> + Send + Sync + 'static,
        BB: FnOnce() -> B,
    {
        let config_path = config_dir.join(wcore::paths::CONFIG_FILE);
        let config = NodeConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        let backend = build_backend();
        let build_provider: BuildProvider<P> = Arc::new(build_provider);
        let node = Node::build(
            &config,
            config_dir,
            shutdown_tx.clone(),
            backend,
            build_provider,
        )
        .await?;

        Ok(NodeHandle {
            config,
            shutdown_tx,
            node,
        })
    }
}

pub struct NodeHandle<P: Provider + 'static = DefaultProvider, B: Host + 'static = NodeHost> {
    pub config: NodeConfig,
    pub shutdown_tx: broadcast::Sender<()>,
    pub node: Node<P, B>,
}

impl<P: Provider + 'static, B: Host + 'static> NodeHandle<P, B> {
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }
}

// ── Transport setup helpers ──────────────────────────────────────────

/// Return a per-message callback that spawns a task driving
/// `node.dispatch` and piping its output to `reply`. Shared between the
/// UDS and TCP transports.
fn dispatch_callback<P: Provider + 'static, H: Host + 'static>(
    node: Node<P, H>,
) -> impl Fn(ClientMessage, mpsc::Sender<wcore::protocol::message::ServerMessage>) + Clone + Send + 'static
{
    move |msg, reply| {
        let node = node.clone();
        tokio::spawn(async move {
            let stream = node.dispatch(msg);
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
pub fn setup_socket<P: Provider + 'static, H: Host + 'static>(
    node: Node<P, H>,
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
        dispatch_callback(node),
        socket_shutdown,
    ));

    Ok((resolved_path, join))
}

pub fn setup_tcp<P: Provider + 'static, H: Host + 'static>(
    node: Node<P, H>,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(tokio::task::JoinHandle<()>, u16)> {
    let (std_listener, addr) = transport::tcp::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let tcp_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let join = tokio::spawn(transport::tcp::accept_loop(
        listener,
        dispatch_callback(node),
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

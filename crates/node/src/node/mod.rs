//! Node — the core struct composing runtime, transports, and lifecycle.

use crate::{
    NodeConfig,
    cron::CronStore,
    event_bus::EventBus,
    hook::host::NodeHost,
    node::{
        builder::{BuildProvider, DefaultProvider, build_default_provider},
        event::{NodeEvent, NodeEventSender},
    },
    storage::FsStorage,
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

/// Shared daemon state.
pub struct Node<P: Provider + 'static = DefaultProvider, B: Host + 'static = NodeHost> {
    #[allow(clippy::type_complexity)]
    pub runtime: Arc<RwLock<Arc<Runtime<P, Env<B, FsStorage>>>>>,
    pub(crate) config_dir: PathBuf,
    pub(crate) event_tx: NodeEventSender,
    pub(crate) started_at: std::time::Instant,
    pub(crate) crons: Arc<Mutex<CronStore>>,
    pub(crate) events: Arc<Mutex<EventBus>>,
    pub(crate) build_provider: BuildProvider<P>,
}

impl<P: Provider + 'static, B: Host + 'static> Clone for Node<P, B> {
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

impl Node<DefaultProvider, NodeHost> {
    pub async fn start(config_dir: &Path) -> Result<NodeHandle<DefaultProvider, NodeHost>> {
        Self::start_with(
            config_dir,
            |config: &NodeConfig| build_default_provider(config),
            |event_tx| {
                let (events_tx, _) = broadcast::channel(256);
                NodeHost {
                    event_tx,
                    pending_asks: Arc::new(Mutex::new(std::collections::HashMap::new())),
                    conversation_cwds: Arc::new(Mutex::new(std::collections::HashMap::new())),
                    events_tx,
                    mcp: Arc::new(crate::mcp::McpHandler::empty()),
                }
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
        BB: FnOnce(NodeEventSender) -> B,
    {
        let config_path = config_dir.join(wcore::paths::CONFIG_FILE);
        let config = NodeConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (event_tx, event_rx) = mpsc::unbounded_channel::<NodeEvent>();

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let shutdown_event_tx = event_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
            let _ = shutdown_event_tx.send(NodeEvent::Shutdown);
        });

        let backend = build_backend(event_tx.clone());
        let build_provider: BuildProvider<P> = Arc::new(build_provider);
        let node = Node::build(
            &config,
            config_dir,
            event_tx.clone(),
            shutdown_tx.clone(),
            backend,
            build_provider,
        )
        .await?;

        let n = node.clone();
        let event_loop_join = tokio::spawn(async move {
            n.handle_events(event_rx).await;
        });

        Ok(NodeHandle {
            config,
            event_tx,
            shutdown_tx,
            node,
            event_loop_join: Some(event_loop_join),
        })
    }
}

pub struct NodeHandle<P: Provider + 'static = DefaultProvider, B: Host + 'static = NodeHost> {
    pub config: NodeConfig,
    pub event_tx: NodeEventSender,
    pub shutdown_tx: broadcast::Sender<()>,
    pub node: Node<P, B>,
    event_loop_join: Option<tokio::task::JoinHandle<()>>,
}

impl<P: Provider + 'static, B: Host + 'static> NodeHandle<P, B> {
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    pub async fn shutdown(mut self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        if let Some(join) = self.event_loop_join.take() {
            join.await?;
        }
        Ok(())
    }
}

// ── Transport setup helpers ──────────────────────────────────────────

#[cfg(unix)]
pub fn setup_socket(
    shutdown_tx: &broadcast::Sender<()>,
    event_tx: &NodeEventSender,
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
            let _ = socket_tx.send(NodeEvent::Message { msg, reply });
        },
        socket_shutdown,
    ));

    Ok((resolved_path, join))
}

pub fn setup_tcp(
    shutdown_tx: &broadcast::Sender<()>,
    event_tx: &NodeEventSender,
) -> Result<(tokio::task::JoinHandle<()>, u16)> {
    let (std_listener, addr) = transport::tcp::bind()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    tracing::info!("daemon listening on tcp://{addr}");

    let tcp_shutdown = bridge_shutdown(shutdown_tx.subscribe());
    let tcp_tx = event_tx.clone();
    let join = tokio::spawn(transport::tcp::accept_loop(
        listener,
        move |msg, reply| {
            let _ = tcp_tx.send(NodeEvent::Message { msg, reply });
        },
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

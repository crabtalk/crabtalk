//! Node — the core struct composing runtime, transports, and lifecycle.

use crate::{NodeConfig, storage::FsStorage};
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
    cron::CronStore,
    event::EventBus,
    host::NodeEnv,
};

/// Per-conversation working directory overrides.
pub type ConversationCwds = Arc<Mutex<HashMap<u64, PathBuf>>>;

/// Pending ask_user oneshots (shared with AskUserHook and protocol layer).
pub type PendingAsks = Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>;

pub mod builder;
pub mod cron;
pub mod event;
pub mod hook;
pub mod host;

/// Config binding for a node.
pub struct NodeCfg<P: Provider + 'static = DefaultProvider> {
    _marker: std::marker::PhantomData<P>,
}

impl<P: Provider + 'static> runtime::Config for NodeCfg<P> {
    type Storage = FsStorage;
    type Provider = P;
    type Env = NodeEnv;
}

/// Shared runtime handle.
pub type SharedRuntime<P> = Arc<RwLock<Arc<Runtime<NodeCfg<P>>>>>;

/// Shared daemon state.
pub struct Node<P: Provider + 'static = DefaultProvider> {
    pub runtime: SharedRuntime<P>,
    /// Composite hook owning all sub-hooks and shared state.
    pub hook: Arc<hook::NodeHook>,
    pub(crate) config_dir: PathBuf,
    pub(crate) started_at: std::time::Instant,
    pub(crate) crons: Arc<Mutex<CronStore<P>>>,
    pub(crate) events: Arc<std::sync::Mutex<EventBus>>,
    pub(crate) build_provider: BuildProvider<P>,
    pub(crate) mcp: Arc<crate::mcp::McpHandler>,
    /// Per-conversation CWD overrides (shared with OsHook + DelegateHook + NodeEnv).
    pub conversation_cwds: ConversationCwds,
    /// Pending ask_user replies (shared with AskUserHook + protocol layer).
    pub pending_asks: PendingAsks,
    /// Bash approval sender (preserved across reloads).
    pub(crate) approval_tx: crate::hooks::os::ApprovalTx,
    /// Bash approval requests — take once to handle in the app layer.
    pub approvals: Arc<std::sync::Mutex<Option<mpsc::Receiver<crate::hooks::os::ApprovalRequest>>>>,
}

impl<P: Provider + 'static> Clone for Node<P> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            hook: self.hook.clone(),
            config_dir: self.config_dir.clone(),
            started_at: self.started_at,
            crons: self.crons.clone(),
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
            mcp: self.mcp.clone(),
            conversation_cwds: self.conversation_cwds.clone(),
            pending_asks: self.pending_asks.clone(),
            approval_tx: self.approval_tx.clone(),
            approvals: self.approvals.clone(),
        }
    }
}

impl Node<DefaultProvider> {
    pub async fn start(config_dir: &Path) -> Result<NodeHandle<DefaultProvider>> {
        let config_path = config_dir.join(wcore::paths::CONFIG_FILE);
        let config = NodeConfig::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let build_provider: BuildProvider<DefaultProvider> =
            Arc::new(|config: &NodeConfig| build_default_provider(config));

        let node = Node::build(&config, config_dir, shutdown_tx.clone(), build_provider).await?;

        Ok(NodeHandle {
            config,
            shutdown_tx,
            node,
        })
    }
}

pub struct NodeHandle<P: Provider + 'static = DefaultProvider> {
    pub config: NodeConfig,
    pub shutdown_tx: broadcast::Sender<()>,
    pub node: Node<P>,
}

impl<P: Provider + 'static> NodeHandle<P> {
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
    node: Node<P>,
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
pub fn setup_socket<P: Provider + 'static>(
    node: Node<P>,
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

pub fn setup_tcp<P: Provider + 'static>(
    node: Node<P>,
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

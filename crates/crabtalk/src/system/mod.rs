//! CrabTalk — the core struct composing runtime, hooks, and protocol.

use crate::bridge::ClientBridge;
use crate::llm::Provider;
use anyhow::Result;
use runtime::Runtime;
use schema::storage::Storage;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{RwLock, broadcast};
pub use transport::{bridge_shutdown, setup_tcp};
use {
    builder::{BuildProvider, build_default_provider},
    event::EventBus,
    host::SystemEnv,
    provider::DefaultProvider,
};

#[cfg(unix)]
pub use transport::setup_socket;

pub mod builder;
pub mod event;
pub mod host;
pub mod provider;
mod transport;

/// Live-reloadable handle to the runtime. The outer `RwLock` lets
/// `CrabTalk::reload()` swap the inner `Arc<Runtime>` without
/// invalidating handles held by hooks; the inner `Arc` is so callers
/// can snapshot and release the lock in one shot.
pub type RuntimeHandle<P, S> = Arc<RwLock<Arc<Runtime<SystemCfg<P, S>>>>>;

/// Config binding for the runtime.
pub struct SystemCfg<P: Provider + 'static, S: Storage> {
    _marker: std::marker::PhantomData<(P, S)>,
}

impl<P: Provider + 'static, S: Storage> runtime::Config for SystemCfg<P, S> {
    type Storage = S;
    type Provider = P;
    type Env = SystemEnv;
}

/// Core crabtalk instance — runtime, hooks, and protocol.
pub struct CrabTalk<P: Provider + 'static, S: Storage> {
    pub runtime: RuntimeHandle<P, S>,
    /// Root hook owning all sub-hooks and shared state.
    pub hook: Arc<hooks::Hooks>,
    pub(crate) config_dir: PathBuf,
    pub(crate) started_at: std::time::Instant,
    pub(crate) events: Arc<parking_lot::Mutex<EventBus>>,
    pub(crate) build_provider: BuildProvider<P>,
    pub(crate) mcp: Arc<mcp::McpHandler>,
    /// Forwards client-tool dispatches to the connected client.
    pub(crate) bridge: Arc<ClientBridge>,
}

impl<P: Provider + 'static, S: Storage> Clone for CrabTalk<P, S> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            hook: self.hook.clone(),
            config_dir: self.config_dir.clone(),
            started_at: self.started_at,
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
            mcp: self.mcp.clone(),
            bridge: self.bridge.clone(),
        }
    }
}

impl<P: Provider + 'static, S: Storage> CrabTalk<P, S> {
    /// Start against a caller-supplied provider and storage. These are the
    /// seams an embedder uses to bring its own keys rather than the single
    /// configured endpoint, and its own persistence rather than the
    /// filesystem. This crate builds neither — it is handed both.
    pub async fn start_with(
        config_dir: &Path,
        storage: Arc<S>,
        build_provider: BuildProvider<P>,
    ) -> Result<CrabTalkHandle<P, S>> {
        let config_path = config_dir.join(schema::paths::CONFIG_FILE);
        let config = schema::Config::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let inner = CrabTalk::build(&config, config_dir, storage, build_provider).await?;

        Ok(CrabTalkHandle {
            config,
            shutdown_tx,
            inner,
        })
    }
}

impl<S: Storage> CrabTalk<DefaultProvider, S> {
    /// Start against the configured endpoint, with the caller's storage.
    pub async fn start(
        config_dir: &Path,
        storage: Arc<S>,
    ) -> Result<CrabTalkHandle<DefaultProvider, S>> {
        let build_provider: BuildProvider<DefaultProvider> =
            Arc::new(|config: &schema::Config, models: &[String]| {
                build_default_provider(config, models)
            });

        Self::start_with(config_dir, storage, build_provider).await
    }
}

pub struct CrabTalkHandle<P: Provider + 'static, S: Storage> {
    pub config: schema::Config,
    pub shutdown_tx: broadcast::Sender<()>,
    pub inner: CrabTalk<P, S>,
}

impl<P: Provider + 'static, S: Storage> CrabTalkHandle<P, S> {
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }
}

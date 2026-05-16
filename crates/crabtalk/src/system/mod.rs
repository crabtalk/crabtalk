//! CrabTalk — the core struct composing runtime, hooks, and protocol.

use crate::{hooks, storage::FsStorage};
use anyhow::Result;
use crabllm_core::Provider;
use runtime::Runtime;
use std::collections::HashMap;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};
pub use transport::{bridge_shutdown, setup_tcp};
use {
    builder::{BuildProvider, DefaultProvider, build_default_provider},
    event::EventBus,
    host::SystemEnv,
};

#[cfg(unix)]
pub use transport::setup_socket;

pub mod builder;
pub mod event;
pub mod hook;
pub mod host;
mod transport;

/// Pending ask_user oneshots (shared with AskUserHook and protocol layer).
pub type PendingAsks = Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>;

/// Shared runtime handle.
pub type SharedRuntime<P> = Arc<RwLock<Arc<Runtime<SystemCfg<P>>>>>;

/// Config binding for the runtime.
pub struct SystemCfg<P: Provider + 'static = DefaultProvider> {
    _marker: std::marker::PhantomData<P>,
}

impl<P: Provider + 'static> runtime::Config for SystemCfg<P> {
    type Storage = FsStorage;
    type Provider = P;
    type Env = SystemEnv;
}

/// Core crabtalk instance — runtime, hooks, and protocol.
pub struct CrabTalk<P: Provider + 'static = DefaultProvider> {
    pub runtime: SharedRuntime<P>,
    /// Composite hook owning all sub-hooks and shared state.
    pub hook: Arc<hook::CompositeHook>,
    pub(crate) config_dir: PathBuf,
    pub(crate) started_at: std::time::Instant,
    pub(crate) events: Arc<parking_lot::Mutex<EventBus>>,
    pub(crate) build_provider: BuildProvider<P>,
    pub(crate) mcp: Arc<mcp::McpHandler>,
    /// Forwards OS-tool dispatches to the connected client.
    pub(crate) client_tools_hook: Arc<hooks::client_tools::ClientToolHook>,
    /// Ask-user hook — owns pending ask oneshots.
    pub(crate) ask_hook: Arc<hooks::ask_user::AskUserHook>,
}

impl<P: Provider + 'static> Clone for CrabTalk<P> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            hook: self.hook.clone(),
            config_dir: self.config_dir.clone(),
            started_at: self.started_at,
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
            mcp: self.mcp.clone(),
            client_tools_hook: self.client_tools_hook.clone(),
            ask_hook: self.ask_hook.clone(),
        }
    }
}

impl CrabTalk<DefaultProvider> {
    pub async fn start(config_dir: &Path) -> Result<CrabTalkHandle<DefaultProvider>> {
        let config_path = config_dir.join(wcore::paths::CONFIG_FILE);
        let config = wcore::Config::load(&config_path)?;
        tracing::info!("loaded configuration from {}", config_path.display());

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let build_provider: BuildProvider<DefaultProvider> =
            Arc::new(|config: &wcore::Config, models: &[String]| {
                build_default_provider(config, models)
            });

        let ct = CrabTalk::build(&config, config_dir, build_provider).await?;

        Ok(CrabTalkHandle {
            config,
            shutdown_tx,
            inner: ct,
        })
    }
}

pub struct CrabTalkHandle<P: Provider + 'static = DefaultProvider> {
    pub config: wcore::Config,
    pub shutdown_tx: broadcast::Sender<()>,
    pub inner: CrabTalk<P>,
}

impl<P: Provider + 'static> CrabTalkHandle<P> {
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }
}

//! CrabTalk — the core struct composing runtime, harnesses, and protocol.

use crate::{Config, harness::HarnessRegistry, llm::Provider, system::builder::BuildProvider};
use anyhow::Result;
use runtime::{Harness, Runtime, Sessions};
use std::sync::Arc;
use store::interface::Backend;
use tokio::sync::{RwLock, broadcast};
use {
    builder::build_default_provider, event::EventBus, host::SystemEnv, provider::DefaultProvider,
};

pub mod builder;
pub mod event;
pub mod host;
pub mod provider;

/// Handle to the runtime. The outer `RwLock` keeps the inner
/// `Arc<Runtime>` swappable without invalidating handles held by
/// harnesses; the inner `Arc` is so callers can snapshot and release
/// the lock in one shot.
pub type RuntimeHandle<P, S> = Arc<RwLock<Arc<Runtime<SystemCfg<P, S>>>>>;

/// Config binding for the runtime.
pub struct SystemCfg<P: Provider + 'static, S: Backend> {
    _marker: std::marker::PhantomData<(P, S)>,
}

impl<P: Provider + 'static, S: Backend> runtime::Config for SystemCfg<P, S> {
    type Storage = S;
    type Provider = P;
    type Env = SystemEnv<S>;
}

/// Core crabtalk instance — runtime, harnesses, and protocol.
pub struct CrabTalk<P: Provider + 'static, S: Backend> {
    pub runtime: RuntimeHandle<P, S>,
    /// Root registry owning all harnesses and shared state.
    pub registry: Arc<HarnessRegistry<S>>,
    pub(crate) started_at: std::time::Instant,
    pub(crate) events: Arc<parking_lot::Mutex<EventBus<S>>>,
    pub(crate) build_provider: BuildProvider<P>,
    /// Live sessions, owned here so they outlive any runtime swap.
    pub(crate) sessions: Arc<Sessions>,
}

impl<P: Provider + 'static, S: Backend> Clone for CrabTalk<P, S> {
    fn clone(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            registry: self.registry.clone(),
            started_at: self.started_at,
            events: self.events.clone(),
            build_provider: Arc::clone(&self.build_provider),
            sessions: self.sessions.clone(),
        }
    }
}

impl<P: Provider + 'static, S: Backend> CrabTalk<P, S> {
    /// Start against a caller-supplied configuration, provider, storage, and
    /// harnesses. These are the seams an embedder uses to bring its own
    /// settings rather than a file on this machine, its own keys rather than
    /// the single configured endpoint, its own persistence rather than the
    /// filesystem, and its own capabilities alongside the built-ins. This
    /// crate builds none of them — it is handed all four.
    pub async fn start_with(
        config: Config,
        storage: Arc<S>,
        build_provider: BuildProvider<P>,
        harnesses: Vec<Arc<dyn Harness>>,
    ) -> Result<CrabTalkHandle<P, S>> {
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let inner = CrabTalk::build(&config, storage, build_provider, harnesses).await?;

        Ok(CrabTalkHandle {
            config,
            shutdown_tx,
            inner,
        })
    }
}

impl<S: Backend> CrabTalk<DefaultProvider, S> {
    /// Start against the configured endpoint, with the caller's storage.
    pub async fn start(
        config: Config,
        storage: Arc<S>,
    ) -> Result<CrabTalkHandle<DefaultProvider, S>> {
        let build_provider: BuildProvider<DefaultProvider> =
            Arc::new(|config: &store::Config, models: &[String]| {
                build_default_provider(config, models)
            });

        Self::start_with(config, storage, build_provider, vec![]).await
    }
}

pub struct CrabTalkHandle<P: Provider + 'static, S: Backend> {
    pub config: Config,
    pub shutdown_tx: broadcast::Sender<()>,
    pub inner: CrabTalk<P, S>,
}

impl<P: Provider + 'static, S: Backend> CrabTalkHandle<P, S> {
    pub async fn wait_until_ready(&self) -> Result<()> {
        Ok(())
    }

    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }
}

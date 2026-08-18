//! CrabTalk construction and lifecycle methods.

use crate::llm::Provider;
use crate::{
    CrabTalk,
    system::RuntimeHandle,
    system::{event, host::SystemEnv, provider::DefaultProvider},
};
use anyhow::Result;
use crabtalk_berm::HarnessHook;
use mcp::McpHandler;
use proto::server::Server;
use runtime::agent::Model;
use runtime::harness::{EventSink, Hooks, McpHook, MemoryHook};
use runtime::{Harness, Runtime, Sessions};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
};
use store::interface::Backend;
use tokio::sync::{RwLock, broadcast};

/// Build the LLM `Model<P>` given the config and the list of models
/// advertised by the endpoint (fetched from `/v1/models` at startup).
pub type BuildProvider<P> =
    Arc<dyn Fn(&store::Config, &[String]) -> Result<runtime::agent::Model<P>> + Send + Sync>;

pub fn build_default_provider(
    config: &store::Config,
    models: &[String],
) -> Result<Model<DefaultProvider>> {
    build_providers(config, models)
}

impl<P: Provider + 'static, S: Backend> CrabTalk<P, S> {
    pub(crate) async fn build(
        config: &store::Config,
        config_dir: &Path,
        storage: Arc<S>,
        build_provider: BuildProvider<P>,
    ) -> Result<Self> {
        let runtime_once: Arc<OnceLock<RuntimeHandle<P, S>>> = Arc::new(OnceLock::new());
        let protocol: Arc<OnceLock<crabtalk_berm::Dispatch>> = Arc::new(OnceLock::new());
        let hooks = Hooks::new(Arc::new(parking_lot::RwLock::new(BTreeMap::new())));
        let (runtime, mcp, hooks) =
            Self::build_all(config, storage, &build_provider, protocol.clone(), hooks).await?;
        let shared_runtime: RuntimeHandle<P, S> = Arc::new(RwLock::new(Arc::new(runtime)));
        runtime_once
            .set(shared_runtime.clone())
            .unwrap_or_else(|_| panic!("runtime already initialized"));

        let sessions = Arc::new(Sessions::new(
            config.cache.sessions.map(|mb| mb * 1024 * 1024),
        ));

        let fire_runtime = shared_runtime.clone();
        let fire_sessions = sessions.clone();
        let fire: event::FireCallback = Arc::new(move |sub, payload| {
            let runtime = fire_runtime.clone();
            let sessions = fire_sessions.clone();
            let target_agent = sub.target_agent;
            let source = sub.source.clone();
            let payload = payload.to_owned();
            tokio::spawn(async move {
                let rt = runtime.read().await.clone();
                let sender = format!("event:{source}");
                let session = match sessions.get_or_create(&rt, &target_agent, &sender).await {
                    Ok((_, session)) => session,
                    Err(e) => {
                        tracing::warn!("event fire: session(agent='{target_agent}'): {e}");
                        return;
                    }
                };
                if let Err(e) = rt.send_to(&session, &payload, &sender, None, vec![]).await {
                    tracing::warn!("event fire: send_to(agent='{target_agent}'): {e}");
                }
            });
        });
        let event_bus = event::EventBus::load(config_dir.to_path_buf(), fire);
        let events = Arc::new(parking_lot::Mutex::new(event_bus));

        {
            let events_for_sink = events.clone();
            let sink: EventSink = Arc::new(move |source: &str, payload: &str| {
                events_for_sink.lock().publish(source, payload);
            });
            hooks.set_event_sink(sink);
        }

        let daemon = Self {
            runtime: shared_runtime,
            hook: hooks,
            config_dir: config_dir.to_path_buf(),
            started_at: std::time::Instant::now(),
            events,
            build_provider,
            mcp,
            sessions,
        };
        Self::connect_protocol(&protocol, daemon.clone());
        Ok(daemon)
    }

    pub async fn reload(&self) -> Result<()> {
        let config = store::Config::load(&self.config_dir.join(store::CONFIG_FILE))?;
        let runtime_once: Arc<OnceLock<RuntimeHandle<P, S>>> = Arc::new(OnceLock::new());
        runtime_once
            .set(self.runtime.clone())
            .unwrap_or_else(|_| panic!("runtime_once already set"));

        let hooks = Hooks::new(self.hook.scopes.clone());
        // Reload rebuilds the runtime around the same store — the backend
        // was the caller's choice at startup and isn't reconsidered here.
        let storage = self.runtime.read().await.storage().clone();

        let protocol: Arc<OnceLock<crabtalk_berm::Dispatch>> = Arc::new(OnceLock::new());
        let (new_runtime, _mcp, new_hook) = Self::build_all(
            &config,
            storage,
            &self.build_provider,
            protocol.clone(),
            hooks,
        )
        .await?;
        Self::connect_protocol(&protocol, self.clone());
        {
            let events_for_sink = self.events.clone();
            let sink: EventSink = Arc::new(move |source: &str, payload: &str| {
                events_for_sink.lock().publish(source, payload);
            });
            new_hook.set_event_sink(sink);
        }
        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("configuration reloaded");
        Ok(())
    }

    /// Open the protocol door for harnesses, now that there is something
    /// behind it.
    fn connect_protocol(protocol: &OnceLock<crabtalk_berm::Dispatch>, daemon: Self) {
        let dispatch: crabtalk_berm::Dispatch = Arc::new(move |msg| {
            let daemon = daemon.clone();
            Box::pin(async move {
                use futures_util::StreamExt;
                daemon.dispatch(msg).collect::<Vec<_>>().await
            })
        });
        let _ = protocol.set(dispatch);
    }

    /// Build Hooks, SystemEnv, and Runtime in one shot.
    async fn build_all(
        config: &store::Config,
        storage: Arc<S>,
        build_provider: &BuildProvider<P>,
        protocol: Arc<OnceLock<crabtalk_berm::Dispatch>>,
        mut hooks: Hooks,
    ) -> Result<(
        Runtime<crate::system::SystemCfg<P, S>>,
        Arc<McpHandler>,
        Arc<Hooks>,
    )> {
        // Ask the endpoint what it serves; an empty list is survivable, so a
        // failure only warns and the next reload retries.
        let models = match config.llm.kind.is_none() && config.llm.base_url.is_empty() {
            true => {
                tracing::warn!("no llm.base_url configured in config.toml — model list is empty");
                Vec::new()
            }
            false => DefaultProvider::from(&config.llm).model_ids().await,
        };
        let default_model = models.first().cloned().unwrap_or_default();
        Self::scaffold(&storage, &default_model).await?;

        let model = build_provider(config, &models)?;
        let mcp_handler: Arc<McpHandler> = Arc::new(McpHandler::new(
            std::time::Duration::from_secs(config.mcp.idle_timeout),
        ));
        mcp_handler.spawn_reaper();
        Self::register_hooks(
            &mut hooks,
            storage.clone(),
            mcp_handler.clone(),
            config.env.clone(),
            protocol,
        )?;
        let hooks = Arc::new(hooks);

        let (events_tx, _) = broadcast::channel(256);
        let env = Arc::new(SystemEnv {
            events_tx,
            hook: hooks.clone(),
        });

        let mut tools = runtime::ToolRegistry::new();
        for schema in Harness::schema(hooks.as_ref()) {
            tools.insert(schema);
        }
        let runtime = Runtime::new(model, env, storage, tools);
        runtime.set_models(models);
        let runtime = runtime;
        Ok((runtime, mcp_handler, hooks))
    }

    fn register_hooks(
        hooks: &mut Hooks,
        storage: Arc<S>,
        mcp_handler: Arc<McpHandler>,
        env_overlay: BTreeMap<String, String>,
        protocol: Arc<OnceLock<crabtalk_berm::Dispatch>>,
    ) -> Result<()> {
        hooks.register_hook("memory", Arc::new(MemoryHook::new(storage)));
        hooks.register_hook("mcp", Arc::new(McpHook::new(mcp_handler, env_overlay)));

        // No agents are pre-loaded: a harness is acquired for the agent
        // that is running, on the run, through `on_resolve_agent`.
        // Loading every agent's images at startup is what made residency
        // here proportional to how many agents exist rather than how many
        // are working.
        match HarnessHook::new(protocol) {
            Ok(harnesses) => hooks.register_hook("harness", Arc::new(harnesses)),
            Err(error) => tracing::warn!("harness engine unavailable: {error:#}"),
        }

        Ok(())
    }

    /// Seed the built-in `crab` agent on a fresh install and point the
    /// install's default at it.
    ///
    /// First-run policy, not persistence: it composes three interface
    /// calls and has nothing backend-specific in it, so making every
    /// backend implement onboarding would be duplicating this.
    async fn scaffold(storage: &Arc<S>, default_model: &str) -> Result<()> {
        if !storage.agent_ids().await?.is_empty() {
            return Ok(());
        }
        let crab = store::AgentConfig::crab(default_model);
        storage.upsert_agent(&crab).await?;
        storage.set_default_agent(&crab.id).await
    }
}

fn build_providers(config: &store::Config, models: &[String]) -> Result<Model<DefaultProvider>> {
    let llm = &config.llm;
    tracing::info!(
        "llm endpoint registered — {} models from {}",
        models.len(),
        llm.base_url
    );
    Ok(Model::new(DefaultProvider::from(llm)))
}

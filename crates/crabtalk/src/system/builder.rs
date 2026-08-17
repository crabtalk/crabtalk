//! CrabTalk construction and lifecycle methods.

use crate::llm::Provider;
use crate::{
    CrabTalk,
    bridge::ClientBridge,
    system::RuntimeHandle,
    system::{event, host::SystemEnv, provider::DefaultProvider},
};
use anyhow::Result;
use crabtalk_berm::HarnessHook;
use hooks::{EventSink, Hooks, McpHook, Memory, MemoryHook};
use mcp::McpHandler;
use proto::server::Server;
use runtime::{Harness, Runtime};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
};
use tokio::sync::{RwLock, broadcast};
use wcore::{ResolvedDirs, model::Model, resolve_dirs, storage::Storage};

/// Build the LLM `Model<P>` given the config and the list of models
/// advertised by the endpoint (fetched from `/v1/models` at startup).
pub type BuildProvider<P> =
    Arc<dyn Fn(&wcore::Config, &[String]) -> Result<wcore::model::Model<P>> + Send + Sync>;

pub fn build_default_provider(
    config: &wcore::Config,
    models: &[String],
) -> Result<Model<DefaultProvider>> {
    build_providers(config, models)
}

impl<P: Provider + 'static, S: Storage> CrabTalk<P, S> {
    pub(crate) async fn build(
        config: &wcore::Config,
        config_dir: &Path,
        storage: Arc<S>,
        build_provider: BuildProvider<P>,
    ) -> Result<Self> {
        let runtime_once: Arc<OnceLock<RuntimeHandle<P, S>>> = Arc::new(OnceLock::new());
        // Harnesses load before the daemon that answers their protocol calls
        // exists, so they are handed the door rather than the server, and it
        // opens once there is something behind it.
        let protocol: Arc<OnceLock<crabtalk_berm::Dispatch>> = Arc::new(OnceLock::new());

        let hooks = Hooks::new(Arc::new(parking_lot::RwLock::new(BTreeMap::new())));

        let (runtime, mcp, hooks, bridge) = Self::build_all(
            config,
            config_dir,
            storage,
            &build_provider,
            protocol.clone(),
            hooks,
        )
        .await?;
        let shared_runtime: RuntimeHandle<P, S> = Arc::new(RwLock::new(Arc::new(runtime)));
        runtime_once
            .set(shared_runtime.clone())
            .unwrap_or_else(|_| panic!("runtime already initialized"));

        // Rebuild the session search index in the background — it
        // does N file reads per persisted session, which on real
        // disks can take seconds to tens of seconds at scale. Until
        // the rebuild completes, `search_sessions` returns whatever
        // subset has already been indexed (live appends index
        // immediately, so new work is always findable).
        {
            let rebuild_runtime = shared_runtime.clone();
            tokio::spawn(async move {
                let rt = rebuild_runtime.read().await.clone();
                if let Err(e) = rt.rebuild_session_index().await {
                    tracing::warn!("session index rebuild failed: {e}");
                }
            });
        }

        let fire_runtime = shared_runtime.clone();
        let fire: event::FireCallback = Arc::new(move |sub, payload| {
            let runtime = fire_runtime.clone();
            let target_agent = sub.target_agent.clone();
            let source = sub.source.clone();
            let payload = payload.to_owned();
            tokio::spawn(async move {
                let rt = runtime.read().await.clone();
                let sender = format!("event:{source}");
                let conversation_id = match rt
                    .get_or_create_conversation(&target_agent, &sender)
                    .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(
                            "event fire: get_or_create_conversation(agent='{target_agent}'): {e}"
                        );
                        return;
                    }
                };
                if let Err(e) = rt
                    .send_to(conversation_id, &payload, &sender, None, vec![])
                    .await
                {
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
            bridge,
        };
        Self::connect_protocol(&protocol, daemon.clone());
        Ok(daemon)
    }

    pub async fn reload(&self) -> Result<()> {
        let config = wcore::Config::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
        let runtime_once: Arc<OnceLock<RuntimeHandle<P, S>>> = Arc::new(OnceLock::new());
        runtime_once
            .set(self.runtime.clone())
            .unwrap_or_else(|_| panic!("runtime_once already set"));

        let hooks = Hooks::new(self.hook.scopes.clone());
        // Reload rebuilds the runtime around the same store — the backend
        // was the caller's choice at startup and isn't reconsidered here.
        let storage = self.runtime.read().await.storage().clone();

        let protocol: Arc<OnceLock<crabtalk_berm::Dispatch>> = Arc::new(OnceLock::new());
        let (mut new_runtime, _mcp, new_hook, _bridge) = Self::build_all(
            &config,
            &self.config_dir,
            storage,
            &self.build_provider,
            protocol.clone(),
            hooks,
        )
        .await?;
        Self::connect_protocol(&protocol, self.clone());
        {
            let old_runtime = self.runtime.read().await;
            (**old_runtime).transfer_to(&mut new_runtime).await;
        }
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
    ///
    /// `Server::dispatch` is already the one entry point every client goes
    /// through, so a harness gets the same one rather than a second vocabulary
    /// (RFC 0205). It is handed over as a closure because the trait is not
    /// object-safe, and because `berm` must not depend on the crate
    /// that implements it.
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
        config: &wcore::Config,
        config_dir: &Path,
        storage: Arc<S>,
        build_provider: &BuildProvider<P>,
        protocol: Arc<OnceLock<crabtalk_berm::Dispatch>>,
        mut hooks: Hooks,
    ) -> Result<(
        Runtime<crate::system::SystemCfg<P, S>>,
        Arc<McpHandler>,
        Arc<Hooks>,
        Arc<ClientBridge>,
    )> {
        let dirs = resolve_dirs(config_dir);
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
        storage.scaffold(&default_model).await?;

        let model = build_provider(config, &models)?;
        let mcp_handler: Arc<McpHandler> = Arc::new(McpHandler::new(
            std::time::Duration::from_secs(config.mcp.idle_timeout),
        ));
        mcp_handler.spawn_reaper();
        let bridge = Arc::new(ClientBridge::default());
        let shared_memory = Self::register_hooks(
            &mut hooks,
            storage.clone(),
            config_dir,
            mcp_handler.clone(),
            config.env.clone(),
            protocol,
        )
        .await?;
        let hooks = Arc::new(hooks);

        let (events_tx, _) = broadcast::channel(256);
        let env = Arc::new(SystemEnv {
            events_tx,
            hook: hooks.clone(),
            bridge: bridge.clone(),
        });

        let mut tools = wcore::ToolRegistry::new();
        for schema in Harness::schema(hooks.as_ref()) {
            tools.insert(schema);
        }
        let runtime = Runtime::new(model, env, storage, shared_memory, tools);
        runtime.set_models(models);
        let mut runtime = runtime;
        Self::register_agents(&mut runtime, &dirs).await?;
        Ok((runtime, mcp_handler, hooks, bridge))
    }

    async fn register_hooks(
        hooks: &mut Hooks,
        storage: Arc<S>,
        config_dir: &Path,
        mcp_handler: Arc<McpHandler>,
        env_overlay: BTreeMap<String, String>,
        protocol: Arc<OnceLock<crabtalk_berm::Dispatch>>,
    ) -> Result<runtime::SharedMemory> {
        let memory_wrapper = Memory::open(config_dir.join("memory.db"))?;
        let shared_memory = memory_wrapper.shared();
        let memory = Arc::new(memory_wrapper);

        hooks.register_hook("memory", Arc::new(MemoryHook::new(memory)));

        hooks.register_hook("mcp", Arc::new(McpHook::new(mcp_handler, env_overlay)));

        // Harnesses are loaded here rather than when their agent registers,
        // because the schema catalogue is built from `Harness::schema` before any
        // agent exists — a tool the catalogue never saw is a tool no model is
        // offered. Registering an agent later loads its own through
        // `on_register_agent`.
        match HarnessHook::new(protocol) {
            Ok(harnesses) => {
                for agent in storage.list_agents().await.unwrap_or_default() {
                    harnesses.load(&agent.name, &agent);
                }
                hooks.register_hook("harness", Arc::new(harnesses));
            }
            Err(error) => tracing::warn!("harness engine unavailable: {error:#}"),
        }

        Ok(shared_memory)
    }

    async fn register_agents(
        runtime: &mut Runtime<crate::system::SystemCfg<P, S>>,
        dirs: &ResolvedDirs,
    ) -> Result<()> {
        let stored_agents = runtime.storage().list_agents().await?;
        let stored_names: std::collections::BTreeSet<String> =
            stored_agents.iter().map(|a| a.name.clone()).collect();

        for agent in stored_agents {
            if agent.description.is_empty() {
                tracing::warn!(name = %agent.name, "stored agent has no description — skipping");
                continue;
            }
            if agent.model.is_empty() {
                tracing::warn!(name = %agent.name, "stored agent has no model — skipping");
                continue;
            }
            runtime.add_agent(agent);
        }

        for (name, agent) in &dirs.package_agents {
            if stored_names.contains(name) {
                continue;
            }
            let agent = agent.clone();
            if agent.description.is_empty() {
                tracing::warn!(name = %name, "package agent has no description — skipping");
                continue;
            }
            if agent.model.is_empty() {
                tracing::warn!(name = %name, "package agent has no model — skipping");
                continue;
            }
            runtime.add_agent(agent);
        }

        Ok(())
    }
}

fn build_providers(config: &wcore::Config, models: &[String]) -> Result<Model<DefaultProvider>> {
    let llm = &config.llm;
    tracing::info!(
        "llm endpoint registered — {} models from {}",
        models.len(),
        llm.base_url
    );
    Ok(Model::new(DefaultProvider::from(llm)))
}

//! CrabTalk construction and lifecycle methods.

use crate::llm::Provider;
use crate::llmp::{ProviderRegistry, RemoteProvider};
use crate::{
    CrabTalk,
    bridge::ClientBridge,
    hooks::{Memory, delegate},
    storage::FsStorage,
    system::{SharedRuntime, hook::Hooks},
    system::{event, host::SystemEnv},
};
use anyhow::Result;
use mcp::McpHandler;
use runtime::{Hook, Runtime};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tokio::sync::{RwLock, broadcast};
use wcore::{LlmConfig, ResolvedDirs, model::Model, resolve_dirs, storage::Storage};

pub type DefaultProvider = crate::provider::Retrying<ProviderRegistry<RemoteProvider>>;

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

impl<P: Provider + 'static> CrabTalk<P> {
    pub(crate) async fn build(
        config: &wcore::Config,
        config_dir: &Path,
        build_provider: BuildProvider<P>,
    ) -> Result<Self> {
        let runtime_once: Arc<OnceLock<SharedRuntime<P>>> = Arc::new(OnceLock::new());

        let hooks = Hooks::new(Arc::new(parking_lot::RwLock::new(BTreeMap::new())));

        let (runtime, mcp, hooks, bridge) = Self::build_all(
            config,
            config_dir,
            &build_provider,
            runtime_once.clone(),
            hooks,
        )
        .await?;
        let shared_runtime: SharedRuntime<P> = Arc::new(RwLock::new(Arc::new(runtime)));
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
            let sink: crate::system::hook::EventSink =
                Arc::new(move |source: &str, payload: &str| {
                    events_for_sink.lock().publish(source, payload);
                });
            hooks.set_event_sink(sink);
        }

        Ok(Self {
            runtime: shared_runtime,
            hook: hooks,
            config_dir: config_dir.to_path_buf(),
            started_at: std::time::Instant::now(),
            events,
            build_provider,
            mcp,
            bridge,
        })
    }

    pub async fn reload(&self) -> Result<()> {
        let config = wcore::Config::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
        let runtime_once: Arc<OnceLock<SharedRuntime<P>>> = Arc::new(OnceLock::new());
        runtime_once
            .set(self.runtime.clone())
            .unwrap_or_else(|_| panic!("runtime_once already set"));

        let hooks = Hooks::new(self.hook.scopes.clone());

        let (mut new_runtime, _mcp, new_hook, _bridge) = Self::build_all(
            &config,
            &self.config_dir,
            &self.build_provider,
            runtime_once,
            hooks,
        )
        .await?;
        {
            let old_runtime = self.runtime.read().await;
            (**old_runtime).transfer_to(&mut new_runtime).await;
        }
        {
            let events_for_sink = self.events.clone();
            let sink: crate::system::hook::EventSink =
                Arc::new(move |source: &str, payload: &str| {
                    events_for_sink.lock().publish(source, payload);
                });
            new_hook.set_event_sink(sink);
        }
        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("configuration reloaded");
        Ok(())
    }

    /// Build Hooks, SystemEnv, and Runtime in one shot.
    async fn build_all(
        config: &wcore::Config,
        config_dir: &Path,
        build_provider: &BuildProvider<P>,
        runtime_once: Arc<OnceLock<SharedRuntime<P>>>,
        mut hooks: Hooks,
    ) -> Result<(
        Runtime<crate::system::SystemCfg<P>>,
        Arc<McpHandler>,
        Arc<Hooks>,
        Arc<ClientBridge>,
    )> {
        let dirs = resolve_dirs(config_dir);
        let storage = Self::build_storage(config_dir, &dirs);
        crate::storage::fs::migrate::migrate_settings(storage.as_ref()).await?;
        let models = fetch_models(&config.llm).await;
        let default_model = models.first().cloned().unwrap_or_default();
        storage.scaffold(&default_model).await?;

        let model = build_provider(config, &models)?;
        let mcp_handler: Arc<McpHandler> = Arc::new(McpHandler::empty());
        let bridge = Arc::new(ClientBridge::new());
        let shared_memory = Self::register_hooks(
            &mut hooks,
            storage.clone(),
            config_dir,
            mcp_handler.clone(),
            config.env.clone(),
            runtime_once,
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
        for schema in Hook::schema(hooks.as_ref()) {
            tools.insert(schema);
        }
        let runtime = Runtime::new(model, env, storage, shared_memory, tools);
        runtime.set_models(models);
        let mut runtime = runtime;
        Self::register_agents(&mut runtime, &dirs).await?;
        Ok((runtime, mcp_handler, hooks, bridge))
    }

    fn build_storage(config_dir: &Path, dirs: &ResolvedDirs) -> Arc<FsStorage> {
        let skill_roots: Vec<PathBuf> = dirs
            .skill_dirs
            .iter()
            .filter(|dir| dir.exists())
            .cloned()
            .collect();

        Arc::new(FsStorage::new(
            config_dir.to_path_buf(),
            config_dir.join("sessions"),
            skill_roots,
        ))
    }

    async fn register_hooks(
        hooks: &mut Hooks,
        storage: Arc<FsStorage>,
        config_dir: &Path,
        mcp_handler: Arc<McpHandler>,
        env_overlay: BTreeMap<String, String>,
        runtime_once: Arc<OnceLock<SharedRuntime<P>>>,
    ) -> Result<runtime::SharedMemory> {
        let memory_wrapper = Memory::open(config_dir.join("memory.db"))?;
        let shared_memory = memory_wrapper.shared();
        let memory = Arc::new(memory_wrapper);
        let scopes = hooks.scopes.clone();
        let skills = storage.list_skills().await.unwrap_or_default();

        hooks.register_hook(
            "memory",
            Arc::new(crate::hooks::memory::MemoryHook::new(memory)),
        );

        hooks.register_hook(
            "sessions",
            Arc::new(crate::hooks::sessions::SessionsHook::<P>::new(
                runtime_once.clone(),
            )),
        );

        hooks.register_hook(
            "skill",
            Arc::new(crate::hooks::skill::handler::SkillHook::new(
                skills,
                scopes.clone(),
            )),
        );
        hooks.register_hook(
            "delegate",
            Arc::new(delegate::DelegateHook::<P>::new(runtime_once)),
        );

        hooks.register_hook(
            "mcp",
            Arc::new(crate::hooks::mcp::McpHook::new(mcp_handler, env_overlay)),
        );
        Ok(shared_memory)
    }

    async fn register_agents(
        runtime: &mut Runtime<crate::system::SystemCfg<P>>,
        dirs: &ResolvedDirs,
    ) -> Result<()> {
        let stored_agents = runtime.storage().list_agents().await?;
        let stored_names: std::collections::BTreeSet<String> =
            stored_agents.iter().map(|a| a.name.clone()).collect();

        for agent in stored_agents {
            if agent.system_prompt.is_empty() {
                tracing::warn!(name = %agent.name, "stored agent has no prompt — skipping");
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
            if agent.system_prompt.is_empty() {
                tracing::warn!(name = %name, "package agent has no prompt — skipping");
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
    let provider_cfg = crate::llm::ProviderConfig {
        kind: crate::llm::ProviderKind::Anthropic,
        base_url: (!llm.base_url.is_empty()).then(|| llm.base_url.clone()),
        api_key: (!llm.api_key.is_empty()).then(|| llm.api_key.clone()),
        models: models.to_vec(),
        ..Default::default()
    };

    let mut providers = std::collections::HashMap::new();
    providers.insert("llm".to_owned(), provider_cfg);

    let registry = ProviderRegistry::from_provider_configs(
        &providers,
        &std::collections::HashMap::new(),
        |r| r,
    )?;
    let retrying = crate::provider::Retrying::new(registry);

    tracing::info!(
        "llm endpoint registered — {} models from {}",
        models.len(),
        llm.base_url
    );
    Ok(Model::new(retrying))
}

/// Fetch `/v1/models` from the configured LLM endpoint. Returns an empty
/// list on failure (logged as a warning) so startup proceeds — the next
/// reload will retry.
async fn fetch_models(llm: &LlmConfig) -> Vec<String> {
    if llm.base_url.is_empty() {
        tracing::warn!("no llm.base_url configured in config.toml — model list is empty");
        return Vec::new();
    }
    let url = format!("{}/models", llm.base_url.trim_end_matches('/'));
    let mut req = reqwest::Client::new().get(&url);
    if !llm.api_key.is_empty() {
        req = req.bearer_auth(&llm.api_key);
    }
    match fetch_models_inner(req).await {
        Ok(models) => models,
        Err(e) => {
            tracing::warn!("failed to fetch {url}: {e}");
            Vec::new()
        }
    }
}

async fn fetch_models_inner(req: reqwest::RequestBuilder) -> Result<Vec<String>> {
    let body: serde_json::Value = req.send().await?.error_for_status()?.json().await?;
    Ok(body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

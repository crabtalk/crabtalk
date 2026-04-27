//! Daemon construction and lifecycle methods.

use crate::{
    Daemon, DaemonConfig,
    daemon::{SharedRuntime, hook::DaemonHook},
    daemon::{event, host::DaemonEnv},
    hooks::{Memory, delegate},
    storage::FsStorage,
};
use anyhow::Result;
use crabllm_core::Provider;
use crabllm_provider::{ProviderRegistry, RemoteProvider};
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

/// Build the LLM `Model<P>` given the daemon config and the list of models
/// advertised by the endpoint (fetched from `/v1/models` at startup).
pub type BuildProvider<P> =
    Arc<dyn Fn(&DaemonConfig, &[String]) -> Result<wcore::model::Model<P>> + Send + Sync>;

pub fn build_default_provider(
    config: &DaemonConfig,
    models: &[String],
) -> Result<Model<DefaultProvider>> {
    build_providers(config, models)
}

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        build_provider: BuildProvider<P>,
    ) -> Result<Self> {
        let runtime_once: Arc<OnceLock<SharedRuntime<P>>> = Arc::new(OnceLock::new());

        let cwd = std::env::current_dir().unwrap_or_else(|_| config_dir.to_path_buf());
        let node_hook = DaemonHook::new(Arc::new(parking_lot::RwLock::new(BTreeMap::new())));

        let (runtime, mcp, node_hook, os_hook, ask_hook) = Self::build_all(
            config,
            config_dir,
            &build_provider,
            runtime_once.clone(),
            cwd.clone(),
            node_hook,
            Default::default(),
            Default::default(),
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
                if let Err(e) = rt.send_to(conversation_id, &payload, &sender, None).await {
                    tracing::warn!("event fire: send_to(agent='{target_agent}'): {e}");
                }
            });
        });
        let event_bus = event::EventBus::load(config_dir.to_path_buf(), fire);
        let events = Arc::new(parking_lot::Mutex::new(event_bus));

        {
            let events_for_sink = events.clone();
            let sink: crate::daemon::hook::EventSink =
                Arc::new(move |source: &str, payload: &str| {
                    events_for_sink.lock().publish(source, payload);
                });
            node_hook.set_event_sink(sink);
        }

        Ok(Self {
            runtime: shared_runtime,
            hook: node_hook,
            config_dir: config_dir.to_path_buf(),
            started_at: std::time::Instant::now(),
            events,
            build_provider,
            mcp,
            os_hook,
            ask_hook,
        })
    }

    pub async fn reload(&self) -> Result<()> {
        let config = DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
        let runtime_once: Arc<OnceLock<SharedRuntime<P>>> = Arc::new(OnceLock::new());
        runtime_once
            .set(self.runtime.clone())
            .unwrap_or_else(|_| panic!("runtime_once already set"));

        let cwd = self.runtime.read().await.env.cwd.clone();

        let node_hook = DaemonHook::new(self.hook.scopes.clone());

        let (mut new_runtime, _mcp, new_hook, _, _) = Self::build_all(
            &config,
            &self.config_dir,
            &self.build_provider,
            runtime_once,
            cwd,
            node_hook,
            self.os_hook.conversation_cwds().clone(),
            self.ask_hook.pending_asks().clone(),
        )
        .await?;
        {
            let old_runtime = self.runtime.read().await;
            (**old_runtime).transfer_to(&mut new_runtime).await;
        }
        {
            let events_for_sink = self.events.clone();
            let sink: crate::daemon::hook::EventSink =
                Arc::new(move |source: &str, payload: &str| {
                    events_for_sink.lock().publish(source, payload);
                });
            new_hook.set_event_sink(sink);
        }
        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("daemon reloaded");
        Ok(())
    }

    /// Build DaemonHook, DaemonEnv, and Runtime in one shot.
    #[allow(clippy::too_many_arguments)]
    async fn build_all(
        config: &DaemonConfig,
        config_dir: &Path,
        build_provider: &BuildProvider<P>,
        runtime_once: Arc<OnceLock<SharedRuntime<P>>>,
        cwd: PathBuf,
        mut node_hook: DaemonHook,
        conversation_cwds: crate::daemon::ConversationCwds,
        pending_asks: crate::daemon::PendingAsks,
    ) -> Result<(
        Runtime<crate::daemon::DaemonCfg<P>>,
        Arc<McpHandler>,
        Arc<DaemonHook>,
        Arc<crate::hooks::os::OsHook>,
        Arc<crate::hooks::ask_user::AskUserHook>,
    )> {
        let dirs = resolve_dirs(config_dir);
        let storage = Self::build_storage(config_dir, &dirs);
        let models = fetch_models(&config.llm).await;
        let default_model = models.first().cloned().unwrap_or_default();
        storage.scaffold(&default_model).await?;

        let model = build_provider(config, &models)?;
        let servers = mcp_servers(config, storage.as_ref(), &dirs).await?;
        let mcp_handler: Arc<McpHandler> = Arc::new(McpHandler::load(&servers).await);
        let (os_hook, ask_hook, shared_memory) = Self::register_tools(
            &mut node_hook,
            storage.clone(),
            config_dir,
            mcp_handler.clone(),
            runtime_once,
            cwd.clone(),
            conversation_cwds.clone(),
            pending_asks,
        )
        .await?;
        let node_hook = Arc::new(node_hook);

        let (events_tx, _) = broadcast::channel(256);
        let env = Arc::new(DaemonEnv {
            events_tx,
            cwd,
            conversation_cwds,
            hook: node_hook.clone(),
        });

        let mut tools = wcore::ToolRegistry::new();
        for schema in Hook::schema(node_hook.as_ref()) {
            tools.insert(schema);
        }
        let runtime = Runtime::new(model, env, storage, shared_memory, tools);
        runtime.set_models(models);
        let mut runtime = runtime;
        Self::register_agents(&mut runtime, &dirs).await?;
        Ok((runtime, mcp_handler, node_hook, os_hook, ask_hook))
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

    #[allow(clippy::too_many_arguments)]
    async fn register_tools(
        node_hook: &mut DaemonHook,
        storage: Arc<FsStorage>,
        config_dir: &Path,
        mcp_handler: Arc<McpHandler>,
        runtime_once: Arc<OnceLock<SharedRuntime<P>>>,
        cwd: PathBuf,
        conversation_cwds: crate::daemon::ConversationCwds,
        pending_asks: crate::daemon::PendingAsks,
    ) -> Result<(
        Arc<crate::hooks::os::OsHook>,
        Arc<crate::hooks::ask_user::AskUserHook>,
        runtime::SharedMemory,
    )> {
        let memory_wrapper = Memory::open(config_dir.join("memory.db"))?;
        let shared_memory = memory_wrapper.shared();
        let memory = Arc::new(memory_wrapper);
        let scopes = node_hook.scopes.clone();
        let read_files: crate::hooks::os::ReadFiles = Default::default();
        let mcp_server_list = mcp_handler.cached_list();
        let skills = storage.list_skills().await.unwrap_or_default();

        let os_hook = Arc::new(crate::hooks::os::OsHook::new(
            cwd,
            conversation_cwds.clone(),
            read_files.clone(),
        ));
        node_hook.register_hook("os", os_hook.clone());

        node_hook.register_hook(
            "memory",
            Arc::new(crate::hooks::memory::MemoryHook::new(memory)),
        );

        node_hook.register_hook(
            "sessions",
            Arc::new(crate::hooks::sessions::SessionsHook::<P>::new(
                runtime_once.clone(),
            )),
        );

        node_hook.register_hook(
            "skill",
            Arc::new(crate::hooks::skill::handler::SkillHook::new(
                skills,
                scopes.clone(),
            )),
        );
        node_hook.register_hook(
            "delegate",
            Arc::new(delegate::DelegateHook::<P>::new(
                runtime_once,
                conversation_cwds,
                read_files,
            )),
        );
        let ask_hook = Arc::new(crate::hooks::ask_user::AskUserHook::new(pending_asks));
        node_hook.register_hook("ask_user", ask_hook.clone());

        if !mcp_server_list.is_empty() {
            let mcp_prompt = format!(
                "\n\n<resources>\nMCP servers: {}. Use the mcp tool to list or call tools.\n</resources>",
                mcp_server_list
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            node_hook.register_hook(
                "mcp",
                Arc::new(crate::hooks::mcp::McpHook::new(
                    mcp_handler,
                    scopes,
                    mcp_prompt,
                )),
            );
        }
        Ok((os_hook, ask_hook, shared_memory))
    }

    /// Load agents from storage (canonical) and plugin manifests.
    /// Storage is seeded with the default `crab` agent by
    /// [`Storage::scaffold`] before this runs. Plugin agents only
    /// register if storage doesn't already shadow them by name.
    async fn register_agents(
        runtime: &mut Runtime<crate::daemon::DaemonCfg<P>>,
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

        // Plugin agents are disk-only — never persisted into storage —
        // so updates flow through `crabtalk pull`, not the daemon
        // mutating settings.toml.
        for (name, agent) in &dirs.plugin_agents {
            if stored_names.contains(name) {
                continue;
            }
            let agent = agent.clone();
            if agent.system_prompt.is_empty() {
                tracing::warn!(name = %name, "plugin agent has no prompt — skipping");
                continue;
            }
            if agent.model.is_empty() {
                tracing::warn!(name = %name, "plugin agent has no model — skipping");
                continue;
            }
            runtime.add_agent(agent);
        }

        Ok(())
    }
}

fn build_providers(config: &DaemonConfig, models: &[String]) -> Result<Model<DefaultProvider>> {
    let llm = &config.llm;
    let provider_cfg = crabllm_core::ProviderConfig {
        kind: crabllm_core::ProviderKind::Openai,
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
/// list on failure (logged as a warning) so the daemon still starts — the
/// next reload will retry.
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

async fn mcp_servers(
    config: &DaemonConfig,
    storage: &FsStorage,
    dirs: &ResolvedDirs,
) -> Result<Vec<wcore::McpServerConfig>> {
    let mut merged: BTreeMap<String, wcore::McpServerConfig> = dirs.plugin_mcps.clone();
    for (name, mcp) in storage.list_mcps().await? {
        merged.insert(name, mcp);
    }
    Ok(merged
        .into_values()
        .map(|mut mcp| {
            for (k, v) in &config.env {
                mcp.env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            mcp
        })
        .collect())
}

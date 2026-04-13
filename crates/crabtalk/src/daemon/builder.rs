//! Daemon construction and lifecycle methods.

use crate::{
    Daemon, NodeConfig,
    daemon::{SharedRuntime, hook::DaemonHook},
    daemon::{cron, event, host::DaemonEnv},
    hooks::{Memory, delegate},
    mcp::McpHandler,
    storage::FsStorage,
};
use anyhow::Result;
use crabllm_core::Provider;
use crabllm_provider::{ProviderRegistry, RemoteProvider};
use runtime::{Hook, Runtime};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::{AgentConfig, ResolvedManifest, model::Model, resolve_manifests, storage::Storage};

pub type DefaultProvider = crate::provider::Retrying<ProviderRegistry<RemoteProvider>>;

pub type BuildProvider<P> =
    Arc<dyn Fn(&NodeConfig) -> Result<wcore::model::Model<P>> + Send + Sync>;

pub fn build_default_provider(config: &NodeConfig) -> Result<Model<DefaultProvider>> {
    build_providers(config)
}

pub(crate) const SYSTEM_AGENT: &str = crate::hooks::memory::DEFAULT_SOUL;

/// Build the `AgentConfig` for a single named agent.
pub(crate) fn build_single_agent_config(
    name: &str,
    config: &NodeConfig,
    manifest: &ResolvedManifest,
    storage: &impl Storage,
) -> Result<AgentConfig> {
    let default_model = config
        .system
        .crab
        .model
        .clone()
        .ok_or_else(|| anyhow::anyhow!("system.crab.model is required in config.toml"))?;

    if name == wcore::paths::DEFAULT_AGENT {
        let mut crab = config.system.crab.clone();
        crab.name = wcore::paths::DEFAULT_AGENT.to_owned();
        crab.system_prompt = SYSTEM_AGENT.to_owned();
        if crab.model.is_none() {
            crab.model = Some(default_model);
        }
        return Ok(crab);
    }

    let agent_config = manifest
        .agents
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found in manifest"))?;

    let prompts = wcore::load_agents_dirs(&manifest.agent_dirs)?;
    let prompt_map: BTreeMap<String, String> = prompts.into_iter().collect();
    let prompt = resolve_agent_prompt(storage, agent_config, name, &prompt_map)
        .ok_or_else(|| anyhow::anyhow!("agent '{name}' has no prompt"))?;

    let mut agent = agent_config.clone();
    agent.name = name.to_owned();
    agent.system_prompt = prompt;
    if agent.model.is_none() {
        agent.model = Some(default_model);
    }
    Ok(agent)
}

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn build(
        config: &NodeConfig,
        config_dir: &Path,
        shutdown_tx: broadcast::Sender<()>,
        build_provider: BuildProvider<P>,
    ) -> Result<Self> {
        if let Err(e) = crate::storage::backfill_local_agent_ids(config_dir) {
            tracing::warn!("agent id backfill failed: {e}");
        }

        let runtime_once: Arc<OnceLock<SharedRuntime<P>>> = Arc::new(OnceLock::new());

        let cwd = std::env::current_dir().unwrap_or_else(|_| config_dir.to_path_buf());
        let node_hook = DaemonHook::new(Arc::new(std::sync::RwLock::new(BTreeMap::new())));

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
        let cron_store = cron::CronStore::load(
            config_dir.to_path_buf(),
            shared_runtime.clone(),
            shutdown_tx,
        );
        let crons = Arc::new(Mutex::new(cron_store));
        crons.lock().await.start_all(crons.clone());

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
        let events = Arc::new(std::sync::Mutex::new(event_bus));

        {
            let events_for_sink = events.clone();
            let sink: crate::daemon::hook::EventSink =
                Arc::new(move |source: &str, payload: &str| {
                    events_for_sink
                        .lock()
                        .expect("event bus lock poisoned")
                        .publish(source, payload);
                });
            node_hook.set_event_sink(sink);
        }

        Ok(Self {
            runtime: shared_runtime,
            hook: node_hook,
            config_dir: config_dir.to_path_buf(),
            started_at: std::time::Instant::now(),
            crons,
            events,
            build_provider,
            mcp,
            os_hook,
            ask_hook,
        })
    }

    pub async fn reload(&self) -> Result<()> {
        let config = NodeConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
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
            (**old_runtime)
                .transfer_conversations(&mut new_runtime)
                .await;
        }
        {
            let events_for_sink = self.events.clone();
            let sink: crate::daemon::hook::EventSink =
                Arc::new(move |source: &str, payload: &str| {
                    events_for_sink
                        .lock()
                        .expect("event bus lock poisoned")
                        .publish(source, payload);
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
        config: &NodeConfig,
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
        let (mut manifest, _warnings) = resolve_manifests(config_dir);
        manifest.disabled = config.disabled.clone();
        wcore::filter_disabled_external(&mut manifest.skill_dirs, &manifest.disabled.external);

        let model = build_provider(config)?;
        let servers = mcp_servers(config, &manifest);
        let mcp_handler: Arc<McpHandler> = Arc::new(McpHandler::load(&servers).await);
        let storage = Self::build_storage(config_dir, &manifest);
        let (os_hook, ask_hook) = Self::register_tools(
            &mut node_hook,
            storage.clone(),
            config,
            mcp_handler.clone(),
            runtime_once,
            cwd.clone(),
            conversation_cwds.clone(),
            pending_asks,
        );
        let node_hook = Arc::new(node_hook);

        // Build DaemonEnv.
        let (events_tx, _) = broadcast::channel(256);
        let env = Arc::new(DaemonEnv {
            events_tx,
            cwd,
            conversation_cwds,
            hook: node_hook.clone(),
        });

        // Build tools and Runtime.
        let mut tools = wcore::ToolRegistry::new();
        for schema in Hook::schema(node_hook.as_ref()) {
            tools.insert(schema);
        }
        let mut runtime = Runtime::new(model, env, storage, tools);
        Self::register_agents(&mut runtime, config, config_dir, &manifest)?;
        Ok((runtime, mcp_handler, node_hook, os_hook, ask_hook))
    }

    fn build_storage(config_dir: &Path, manifest: &ResolvedManifest) -> Arc<FsStorage> {
        let skill_roots: Vec<PathBuf> = manifest
            .skill_dirs
            .iter()
            .filter(|dir| dir.exists())
            .cloned()
            .collect();

        Arc::new(FsStorage::new(
            config_dir.to_path_buf(),
            config_dir.join("memory"),
            config_dir.join("sessions"),
            skill_roots,
            manifest.disabled.skills.clone(),
            manifest.agent_dirs.clone(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn register_tools(
        node_hook: &mut DaemonHook,
        storage: Arc<FsStorage>,
        config: &NodeConfig,
        mcp_handler: Arc<McpHandler>,
        runtime_once: Arc<OnceLock<SharedRuntime<P>>>,
        cwd: PathBuf,
        conversation_cwds: crate::daemon::ConversationCwds,
        pending_asks: crate::daemon::PendingAsks,
    ) -> (
        Arc<crate::hooks::os::OsHook>,
        Arc<crate::hooks::ask_user::AskUserHook>,
    ) {
        let memory = Arc::new(Memory::open(config.system.memory.clone(), storage.clone()));
        let scopes = node_hook.scopes.clone();
        let read_files: crate::hooks::os::ReadFiles = Default::default();
        let mcp_server_list = mcp_handler.cached_list();

        let os_hook = Arc::new(crate::hooks::os::OsHook::new(
            cwd,
            conversation_cwds.clone(),
            read_files.clone(),
            config.system.bash.clone(),
        ));
        node_hook.register_hook("os", os_hook.clone());

        node_hook.register_hook(
            "memory",
            Arc::new(crate::hooks::memory::handlers::MemoryHook::new(memory)),
        );

        node_hook.register_hook(
            "skill",
            Arc::new(crate::hooks::skill::handler::SkillHook::new(
                storage,
                scopes.clone(),
            )),
        );
        node_hook.register_hook(
            "delegate",
            Arc::new(delegate::DelegateHook::<P>::new(
                scopes.clone(),
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
                Arc::new(crate::mcp::tool::McpHook::new(
                    mcp_handler,
                    scopes,
                    mcp_prompt,
                )),
            );
        }
        (os_hook, ask_hook)
    }

    fn register_agents(
        runtime: &mut Runtime<crate::daemon::DaemonCfg<P>>,
        config: &NodeConfig,
        config_dir: &Path,
        manifest: &ResolvedManifest,
    ) -> Result<()> {
        if let Err(e) = crate::storage::migrate_local_agent_prompts(
            config_dir,
            manifest,
            runtime.storage().as_ref(),
        ) {
            tracing::warn!("legacy prompt migration failed: {e}");
        }

        let default_model = config
            .system
            .crab
            .model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("system.crab.model is required in config.toml"))?;

        {
            let mut crab = config.system.crab.clone();
            crab.name = wcore::paths::DEFAULT_AGENT.to_owned();
            crab.system_prompt = SYSTEM_AGENT.to_owned();
            if crab.model.is_none() {
                crab.model = Some(default_model.clone());
            }
            runtime.add_agent(crab);
        }

        let prompts = wcore::load_agents_dirs(&manifest.agent_dirs)?;
        let prompt_map: BTreeMap<String, String> = prompts.into_iter().collect();

        for (name, agent_config) in &manifest.agents {
            let Some(prompt) =
                resolve_agent_prompt(runtime.storage().as_ref(), agent_config, name, &prompt_map)
            else {
                tracing::warn!(name, "agent has no prompt — skipping");
                continue;
            };
            let mut agent = agent_config.clone();
            agent.name = name.clone();
            agent.system_prompt = prompt;
            if agent.model.is_none() {
                agent.model = Some(default_model.clone());
            }
            runtime.add_agent(agent);
        }

        for (name, prompt) in &prompt_map {
            if name == wcore::paths::DEFAULT_AGENT || manifest.agents.contains_key(name) {
                continue;
            }
            let mut config = AgentConfig::new(name);
            config.system_prompt = prompt.clone();
            config.model = Some(default_model.clone());
            runtime.add_agent(config);
        }

        Ok(())
    }
}

fn resolve_agent_prompt(
    storage: &impl Storage,
    config: &AgentConfig,
    name: &str,
    prompt_map: &BTreeMap<String, String>,
) -> Option<String> {
    if let Ok(Some(loaded)) = storage.load_agent_by_name(name)
        && !loaded.system_prompt.is_empty()
    {
        return Some(loaded.system_prompt);
    }
    if !config.id.is_nil()
        && let Ok(Some(loaded)) = storage.load_agent(&config.id)
        && !loaded.system_prompt.is_empty()
    {
        return Some(loaded.system_prompt);
    }
    prompt_map.get(name).cloned()
}

fn build_providers(config: &NodeConfig) -> Result<Model<DefaultProvider>> {
    let providers: std::collections::HashMap<String, _> = config
        .provider
        .iter()
        .filter(|(name, _)| !config.disabled.providers.contains(name))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let provider_count = providers.len();
    let model_count: usize = providers.values().map(|def| def.models.len()).sum();

    let registry = ProviderRegistry::from_provider_configs(
        &providers,
        &std::collections::HashMap::new(),
        |r| r,
    )?;
    let retrying = crate::provider::Retrying::new(registry);

    tracing::info!(
        "provider registry initialized — {model_count} models across {provider_count} providers"
    );
    Ok(Model::new(retrying))
}

fn mcp_servers(config: &NodeConfig, manifest: &ResolvedManifest) -> Vec<wcore::McpServerConfig> {
    manifest
        .mcps
        .iter()
        .filter(|(name, _)| !manifest.disabled.mcps.contains(name))
        .map(|(_, mcp)| {
            let mut mcp = mcp.clone();
            for (k, v) in &config.env {
                mcp.env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            mcp
        })
        .collect()
}

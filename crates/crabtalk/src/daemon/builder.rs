//! Daemon construction and lifecycle methods.

use crate::{
    Daemon, DaemonConfig,
    daemon::{SharedRuntime, hook::DaemonHook},
    daemon::{cron, event, host::DaemonEnv},
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
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::{ResolvedDirs, model::Model, resolve_dirs, storage::Storage};

pub type DefaultProvider = crate::provider::Retrying<ProviderRegistry<RemoteProvider>>;

pub type BuildProvider<P> =
    Arc<dyn Fn(&DaemonConfig) -> Result<wcore::model::Model<P>> + Send + Sync>;

pub fn build_default_provider(config: &DaemonConfig) -> Result<Model<DefaultProvider>> {
    build_providers(config)
}

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        shutdown_tx: broadcast::Sender<()>,
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
            crons,
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
        let default_model = first_provider_model(config).ok_or_else(|| {
            anyhow::anyhow!(
                "no model configured — add at least one model under [provider.<name>] in config.toml"
            )
        })?;
        storage.scaffold(&default_model)?;

        let model = build_provider(config)?;
        let servers = mcp_servers(config, storage.as_ref(), &dirs)?;
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
        )?;
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
        let mut runtime = Runtime::new(model, env, storage, shared_memory, tools);
        Self::register_agents(&mut runtime, &dirs)?;
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
    fn register_tools(
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

        let os_hook = Arc::new(crate::hooks::os::OsHook::new(
            cwd,
            conversation_cwds.clone(),
            read_files.clone(),
            storage.clone(),
        ));
        node_hook.register_hook("os", os_hook.clone());

        node_hook.register_hook(
            "memory",
            Arc::new(crate::hooks::memory::MemoryHook::new(
                memory,
                storage.clone(),
            )),
        );

        node_hook.register_hook(
            "topic",
            Arc::new(crate::hooks::topic::TopicHook::<P>::new(
                runtime_once.clone(),
                shared_memory.clone(),
            )),
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
    fn register_agents(
        runtime: &mut Runtime<crate::daemon::DaemonCfg<P>>,
        dirs: &ResolvedDirs,
    ) -> Result<()> {
        let stored_agents = runtime.storage().list_agents()?;
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

/// First model declared by any provider. Used to seed the default
/// crab agent when no agent is stored yet.
fn first_provider_model(config: &DaemonConfig) -> Option<String> {
    config
        .provider
        .values()
        .flat_map(|def| def.models.iter())
        .next()
        .cloned()
}

fn build_providers(config: &DaemonConfig) -> Result<Model<DefaultProvider>> {
    let providers: std::collections::HashMap<String, _> = config
        .provider
        .iter()
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

fn mcp_servers(
    config: &DaemonConfig,
    storage: &dyn Storage,
    dirs: &ResolvedDirs,
) -> Result<Vec<wcore::McpServerConfig>> {
    let mut merged: BTreeMap<String, wcore::McpServerConfig> = dirs.plugin_mcps.clone();
    for (name, mcp) in storage.list_mcps()? {
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

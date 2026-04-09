//! Daemon construction and lifecycle methods.

use crate::{
    Daemon, DaemonConfig,
    config::{ResolvedManifest, resolve_manifests},
    daemon::event::{DaemonEvent, DaemonEventSender},
};
use anyhow::Result;
use crabllm_core::Provider;
use crabllm_provider::{ProviderRegistry, RemoteProvider};
use runtime::{Env, SkillHandler, host::Host, mcp::McpHandler, memory::Memory};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::{AgentConfig, Runtime, ToolRequest, model::Model};

/// The concrete provider type the default daemon uses: a `crabllm`
/// `ProviderRegistry<RemoteProvider>` wrapped in a `Retrying` layer. The
/// registry implements `crabllm_core::Provider` via model-name routing;
/// `Retrying` adds the exponential-backoff loop and per-call timeout that
/// the daemon expects from a production deployment.
///
/// Exposed (pub) so downstream consumers can name the type explicitly,
/// e.g. `Daemon<DefaultProvider, MyHost>` or as a bound for helper
/// functions that thread P through without caring what it is.
pub type DefaultProvider = crate::provider::Retrying<ProviderRegistry<RemoteProvider>>;

/// Closure that builds a `Model<P>` from a `DaemonConfig`. Stored on
/// `Daemon` so `reload()` can call it with the freshly-loaded config.
/// `Arc<dyn Fn>` so `Daemon` remains `Clone` regardless of concrete P.
pub type BuildProvider<P> =
    Arc<dyn Fn(&DaemonConfig) -> Result<wcore::model::Model<P>> + Send + Sync>;

/// Construct the default `Model<DefaultProvider>` from a config.
///
/// This is the function the `Daemon::start` convenience path uses. Apple
/// app and other library consumers supply their own closure with a
/// different return type.
pub fn build_default_provider(config: &DaemonConfig) -> Result<Model<DefaultProvider>> {
    build_providers(config)
}

/// Resolve qualified plugin references in an agent's skill list.
pub(crate) fn resolve_plugin_skills(
    skills: &mut Vec<String>,
    plugin_skill_dirs: &BTreeMap<String, PathBuf>,
) {
    let mut resolved = Vec::new();
    for entry in skills.drain(..) {
        if entry.contains('/') {
            if let Some(dir) = plugin_skill_dirs.get(&entry) {
                match runtime::skill::loader::load_skills_dir(dir) {
                    Ok(registry) => {
                        for skill in &registry.skills {
                            resolved.push(skill.name.clone());
                        }
                    }
                    Err(e) => {
                        tracing::warn!("failed to resolve plugin skills for '{entry}': {e}");
                    }
                }
            } else {
                tracing::warn!("unknown plugin skill reference: '{entry}'");
            }
        } else {
            resolved.push(entry);
        }
    }
    *skills = resolved;
}

pub(crate) const SYSTEM_AGENT: &str = runtime::memory::DEFAULT_SOUL;

/// Build the `AgentConfig` for a single named agent — the same shape that
/// [`load_agents`] produces at startup. Used by agent CRUD for targeted
/// in-memory updates that skip a full runtime rebuild.
///
/// Returns `Err` if the agent has no manifest entry or no prompt file
/// (except for `DEFAULT_AGENT`, which sources its prompt from the
/// `SYSTEM_AGENT` constant and its config from `[system.crab]`).
pub(crate) fn build_single_agent_config(
    name: &str,
    config: &DaemonConfig,
    manifest: &ResolvedManifest,
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

    let prompts = crate::config::load_agents_dirs(&manifest.agent_dirs)?;
    let prompt = prompts
        .into_iter()
        .find(|(stem, _)| stem == name)
        .map(|(_, content)| content)
        .ok_or_else(|| anyhow::anyhow!("agent '{name}' has no prompt file"))?;

    let mut agent = agent_config.clone();
    agent.name = name.to_owned();
    agent.system_prompt = prompt;
    if agent.model.is_none() {
        agent.model = Some(default_model);
    }
    resolve_plugin_skills(&mut agent.skills, &manifest.plugin_skill_dirs);
    Ok(agent)
}

impl<P: Provider + 'static, H: Host + 'static> Daemon<P, H> {
    /// Build a fully-configured [`Daemon`] from the given config, config
    /// directory, event sender, backend, and provider-builder closure.
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: DaemonEventSender,
        shutdown_tx: broadcast::Sender<()>,
        host: H,
        build_provider: BuildProvider<P>,
    ) -> Result<Self> {
        let runtime =
            Self::build_runtime(config, config_dir, &event_tx, host, &build_provider).await?;
        let cron_store = crate::cron::CronStore::load(
            config_dir.join("crons.toml"),
            event_tx.clone(),
            shutdown_tx,
        );
        let crons = Arc::new(Mutex::new(cron_store));
        crons.lock().await.start_all(crons.clone());
        let event_bus =
            crate::event_bus::EventBus::load(config_dir.join("events.toml"), event_tx.clone());
        let events = Arc::new(Mutex::new(event_bus));
        Ok(Self {
            runtime: Arc::new(RwLock::new(Arc::new(runtime))),
            config_dir: config_dir.to_path_buf(),
            event_tx,
            started_at: std::time::Instant::now(),
            crons,
            events,
            build_provider,
        })
    }

    /// Rebuild the runtime from disk and swap it in atomically.
    ///
    /// Clones the backend from the current runtime so shared state
    /// (channels, pending asks) is preserved across reloads. The
    /// provider-builder closure stored on `Daemon` is re-run with the
    /// fresh config to construct the new `Model<P>`.
    pub async fn reload(&self) -> Result<()> {
        let config = DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
        let host = {
            let old_rt = self.runtime.read().await;
            old_rt.hook.host.clone()
        };
        let mut new_runtime = Self::build_runtime(
            &config,
            &self.config_dir,
            &self.event_tx,
            host,
            &self.build_provider,
        )
        .await?;
        {
            let old_runtime = self.runtime.read().await;
            (**old_runtime)
                .transfer_conversations(&mut new_runtime)
                .await;
        }
        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("daemon reloaded");
        Ok(())
    }

    /// Construct a fresh [`Runtime`] from config with the given backend
    /// and provider builder.
    async fn build_runtime(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
        host: H,
        build_provider: &BuildProvider<P>,
    ) -> Result<Runtime<P, Env<H>>> {
        let (mut manifest, _warnings) = resolve_manifests(config_dir);
        manifest.disabled = config.disabled.clone();
        wcore::filter_disabled_external(&mut manifest.skill_dirs, &manifest.disabled.external);
        let model = build_provider(config)?;
        let hook = build_env(config, config_dir, &manifest, host).await?;
        let tool_tx = build_tool_sender(event_tx);
        let mut runtime = Runtime::new(model, hook, Some(tool_tx)).await;
        load_agents(&mut runtime, config, &manifest)?;
        Ok(runtime)
    }
}

/// Construct the provider registry from config, filtering out disabled
/// providers. Returns the registry wrapped in `Retrying` (for retry +
/// timeout) and then in `Model<P>` so the caller can hand it directly to
/// `Runtime::new`.
fn build_providers(config: &DaemonConfig) -> Result<Model<DefaultProvider>> {
    // Filter out disabled providers and convert from BTreeMap to HashMap
    // (crabllm's `from_provider_configs` takes a HashMap).
    let providers: HashMap<String, _> = config
        .provider
        .iter()
        .filter(|(name, _)| !config.disabled.providers.contains(name))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let provider_count = providers.len();
    let model_count: usize = providers.values().map(|def| def.models.len()).sum();

    let registry = ProviderRegistry::from_provider_configs(&providers, &HashMap::new(), |r| r)?;
    let retrying = crate::provider::Retrying::new(registry);

    tracing::info!(
        "provider registry initialized — {model_count} models across {provider_count} providers"
    );
    Ok(Model::new(retrying))
}

/// Build the engine environment with all backends (skills, MCP, memory).
async fn build_env<H: Host>(
    config: &DaemonConfig,
    config_dir: &Path,
    manifest: &ResolvedManifest,
    host: H,
) -> Result<Env<H>> {
    let skills = SkillHandler::load(manifest.skill_dirs.clone(), &manifest.disabled.skills)
        .unwrap_or_else(|e| {
            tracing::warn!("failed to load skills: {e}");
            SkillHandler::default()
        });

    // Inject [env] from config.toml into each MCP's env map, skipping disabled.
    let mcp_servers: Vec<_> = manifest
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
        .collect();
    let mcp_handler = McpHandler::load(&mcp_servers).await;

    // Pluggable persistence backend — filesystem-rooted at the config
    // dir. Memory takes a clone of the same handle that lands on Env so
    // the whole runtime shares one Storage instance.
    let storage: Arc<dyn runtime::Storage> =
        Arc::new(crate::storage::FsStorage::new(config_dir.to_path_buf()));

    let memory = Some(Memory::open(config.system.memory.clone(), storage.clone()));

    let cwd = std::env::current_dir().unwrap_or_else(|_| config_dir.to_path_buf());

    Ok(Env::new(skills, mcp_handler, cwd, memory, storage, host))
}

/// Build a [`ToolSender`] that forwards [`ToolRequest`]s into the daemon
/// event loop as [`DaemonEvent::ToolCall`] variants.
fn build_tool_sender(event_tx: &DaemonEventSender) -> wcore::ToolSender {
    let (tool_tx, mut tool_rx) = tokio::sync::mpsc::unbounded_channel::<ToolRequest>();
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            if event_tx.send(DaemonEvent::ToolCall(req)).is_err() {
                break;
            }
        }
    });
    tool_tx
}

/// Load agents and add them to the runtime.
fn load_agents<P: Provider + 'static, H: Host + 'static>(
    runtime: &mut Runtime<P, Env<H>>,
    config: &DaemonConfig,
    manifest: &ResolvedManifest,
) -> Result<()> {
    let prompts = crate::config::load_agents_dirs(&manifest.agent_dirs)?;
    let prompt_map: BTreeMap<String, String> = prompts.into_iter().collect();

    // The daemon-wide default model. Required because every agent must
    // resolve to a concrete model name at registration time — there is no
    // longer a runtime fallback in the registry.
    let default_model = config
        .system
        .crab
        .model
        .clone()
        .ok_or_else(|| anyhow::anyhow!("system.crab.model is required in config.toml"))?;

    // Built-in crab agent.
    let mut crab_config = config.system.crab.clone();
    crab_config.name = wcore::paths::DEFAULT_AGENT.to_owned();
    crab_config.system_prompt = SYSTEM_AGENT.to_owned();
    runtime.add_agent(crab_config.clone());

    // Sub-agents from manifests.
    for (name, agent_config) in &manifest.agents {
        if name == wcore::paths::DEFAULT_AGENT {
            tracing::warn!(
                "agents.{name} overrides the built-in system agent and will be ignored — \
                 configure it under [system.crab] instead"
            );
            continue;
        }
        let Some(prompt) = prompt_map.get(name) else {
            tracing::warn!("agent '{name}' in manifest has no matching .md file, skipping");
            continue;
        };
        let mut agent = agent_config.clone();
        agent.name = name.clone();
        agent.system_prompt = prompt.clone();
        if agent.model.is_none() {
            agent.model = Some(default_model.clone());
        }
        resolve_plugin_skills(&mut agent.skills, &manifest.plugin_skill_dirs);
        tracing::info!("registered agent '{name}' (thinking={})", agent.thinking);
        runtime.add_agent(agent);
    }

    // Also register agents that have .md files but no manifest entry.
    let default_think = config.system.crab.thinking;
    for (stem, prompt) in &prompt_map {
        if stem == wcore::paths::DEFAULT_AGENT {
            tracing::warn!(
                "agents/{stem}.md shadows the built-in system agent and will be ignored"
            );
            continue;
        }
        if manifest.agents.contains_key(stem) {
            continue;
        }
        let mut agent = AgentConfig::new(stem.as_str());
        agent.system_prompt = prompt.clone();
        agent.thinking = default_think;
        agent.model = Some(default_model.clone());
        tracing::info!("registered agent '{stem}' (defaults, thinking={default_think})");
        runtime.add_agent(agent);
    }

    // Populate per-agent scope maps.
    for agent_config in runtime.agents() {
        runtime
            .hook
            .register_scope(agent_config.name.clone(), &agent_config);
    }

    Ok(())
}

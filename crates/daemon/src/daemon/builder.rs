//! Daemon construction and lifecycle methods.

use crate::{
    Daemon, DaemonConfig,
    config::{ResolvedManifest, resolve_manifests},
    daemon::event::{DaemonEvent, DaemonEventSender},
};
use anyhow::Result;
use model::ProviderRegistry;
use runtime::{Env, SkillHandler, host::Host, mcp::McpHandler, memory::Memory};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::{AgentConfig, Runtime, ToolRequest};

/// Resolve qualified plugin references in an agent's skill list.
fn resolve_plugin_skills(skills: &mut Vec<String>, plugin_skill_dirs: &BTreeMap<String, PathBuf>) {
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

const SYSTEM_AGENT: &str = runtime::memory::DEFAULT_SOUL;

impl<H: Host + 'static> Daemon<H> {
    /// Build a fully-configured [`Daemon`] from the given config, config
    /// directory, event sender, and backend.
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: DaemonEventSender,
        shutdown_tx: broadcast::Sender<()>,
        host: H,
    ) -> Result<Self> {
        let runtime = Self::build_runtime(config, config_dir, &event_tx, host).await?;
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
        })
    }

    /// Rebuild the runtime from disk and swap it in atomically.
    ///
    /// Clones the backend from the current runtime so shared state
    /// (channels, pending asks) is preserved across reloads.
    pub async fn reload(&self) -> Result<()> {
        let config = DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
        let host = {
            let old_rt = self.runtime.read().await;
            old_rt.hook.host.clone()
        };
        let mut new_runtime =
            Self::build_runtime(&config, &self.config_dir, &self.event_tx, host).await?;
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

    /// Construct a fresh [`Runtime`] from config with the given backend.
    async fn build_runtime(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
        host: H,
    ) -> Result<Runtime<ProviderRegistry, Env<H>>> {
        let (mut manifest, _warnings) = resolve_manifests(config_dir);
        manifest.disabled = config.disabled.clone();
        wcore::filter_disabled_external(&mut manifest.skill_dirs, &manifest.disabled.external);
        let manager = build_providers(config, &manifest.disabled)?;
        let hook = build_env(config, config_dir, &manifest, host).await?;
        let tool_tx = build_tool_sender(event_tx);
        let mut runtime = Runtime::new(manager, hook, Some(tool_tx)).await;
        load_agents(&mut runtime, config, &manifest)?;
        Ok(runtime)
    }
}

/// Construct the provider registry from config, filtering out disabled providers.
fn build_providers(
    config: &DaemonConfig,
    disabled: &wcore::config::DisabledItems,
) -> Result<ProviderRegistry> {
    let active_model = config
        .system
        .crab
        .model
        .clone()
        .ok_or_else(|| anyhow::anyhow!("system.crab.model is required in config.toml"))?;
    let providers: BTreeMap<_, _> = config
        .provider
        .iter()
        .filter(|(name, _)| !disabled.providers.contains(name))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let registry = ProviderRegistry::from_providers(active_model, &providers)?;

    tracing::info!(
        "provider registry initialized — active model: {}",
        registry.active_model_name().unwrap_or_default()
    );
    Ok(registry)
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

    let memory = Some(Memory::open(
        config_dir.join("memory"),
        config.system.memory.clone(),
        Box::new(runtime::memory::storage::FsStorage),
    ));

    let cwd = std::env::current_dir().unwrap_or_else(|_| config_dir.to_path_buf());

    Ok(Env::new(skills, mcp_handler, cwd, memory, host))
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
fn load_agents<H: Host + 'static>(
    runtime: &mut Runtime<ProviderRegistry, Env<H>>,
    config: &DaemonConfig,
    manifest: &ResolvedManifest,
) -> Result<()> {
    let prompts = crate::config::load_agents_dirs(&manifest.agent_dirs)?;
    let prompt_map: BTreeMap<String, String> = prompts.into_iter().collect();

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

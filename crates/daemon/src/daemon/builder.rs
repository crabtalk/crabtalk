//! Daemon construction and lifecycle methods.

use crate::hook::DaemonEnv;
use crate::{
    Daemon, DaemonConfig,
    config::{ResolvedManifest, resolve_manifests},
    daemon::event::{DaemonEvent, DaemonEventSender},
    hook::backend::DaemonBackend,
};
use anyhow::Result;
use model::ProviderRegistry;
use runtime::{Env, SkillHandler, mcp::McpHandler, memory::Memory};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::{AgentConfig, Runtime, ToolRequest};

/// Resolve qualified package references in an agent's skill list.
fn resolve_package_skills(
    skills: &mut Vec<String>,
    package_skill_dirs: &BTreeMap<String, PathBuf>,
) {
    let mut resolved = Vec::new();
    for entry in skills.drain(..) {
        if entry.contains('/') {
            if let Some(dir) = package_skill_dirs.get(&entry) {
                match runtime::skill::loader::load_skills_dir(dir) {
                    Ok(registry) => {
                        for skill in registry.skills() {
                            resolved.push(skill.name.clone());
                        }
                    }
                    Err(e) => {
                        tracing::warn!("failed to resolve package skills for '{entry}': {e}");
                    }
                }
            } else {
                tracing::warn!("unknown package skill reference: '{entry}'");
            }
        } else {
            resolved.push(entry);
        }
    }
    *skills = resolved;
}

const SYSTEM_AGENT: &str = runtime::memory::DEFAULT_SOUL;

impl Daemon {
    /// Build a fully-configured [`Daemon`] from the given config, config
    /// directory, and event sender.
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: DaemonEventSender,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Result<Self> {
        let runtime = Self::build_runtime(config, config_dir, &event_tx).await?;
        let cron_store = crate::cron::CronStore::load(
            config_dir.join("crons.toml"),
            event_tx.clone(),
            shutdown_tx,
        );
        let crons = Arc::new(Mutex::new(cron_store));
        crons.lock().await.start_all(crons.clone());
        Ok(Self {
            runtime: Arc::new(RwLock::new(Arc::new(runtime))),
            config_dir: config_dir.to_path_buf(),
            event_tx,
            started_at: std::time::Instant::now(),
            crons,
        })
    }

    /// Rebuild the runtime from disk and swap it in atomically.
    pub async fn reload(&self) -> Result<()> {
        let config = DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
        let mut new_runtime =
            Self::build_runtime(&config, &self.config_dir, &self.event_tx).await?;

        {
            let old_runtime = self.runtime.read().await;
            (**old_runtime).transfer_sessions(&mut new_runtime).await;
        }

        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("daemon reloaded");
        Ok(())
    }

    /// Construct a fresh [`Runtime`] from config.
    async fn build_runtime(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
    ) -> Result<Runtime<ProviderRegistry, DaemonEnv>> {
        let manager = Self::build_providers(config)?;
        let (manifest, _warnings) = resolve_manifests(config_dir);
        let hook = Self::build_hook(config, config_dir, &manifest, event_tx).await?;
        let tool_tx = Self::build_tool_sender(event_tx);
        let mut runtime = Runtime::new(manager, hook, Some(tool_tx)).await;
        Self::load_agents(&mut runtime, config, &manifest)?;
        Ok(runtime)
    }

    /// Construct the provider registry from config.
    fn build_providers(config: &DaemonConfig) -> Result<ProviderRegistry> {
        let active_model = config
            .system
            .crab
            .model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("system.crab.model is required in config.toml"))?;
        let registry = ProviderRegistry::from_providers(active_model.clone(), &config.provider)?;

        tracing::info!(
            "provider registry initialized — active model: {}",
            registry.active_model_name().unwrap_or_default()
        );
        Ok(registry)
    }

    /// Build the daemon hook with all backends (skills, MCP, tasks, memory).
    async fn build_hook(
        config: &DaemonConfig,
        config_dir: &Path,
        manifest: &ResolvedManifest,
        event_tx: &DaemonEventSender,
    ) -> Result<DaemonEnv> {
        let skills = SkillHandler::load(manifest.skill_dirs.clone()).unwrap_or_else(|e| {
            tracing::warn!("failed to load skills: {e}");
            SkillHandler::default()
        });

        // Inject [env] from config.toml into each MCP's env map.
        let mcp_servers: Vec<_> = manifest
            .mcps
            .values()
            .map(|mcp| {
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

        let (events_tx, _) = tokio::sync::broadcast::channel(256);
        let bridge = DaemonBackend {
            event_tx: event_tx.clone(),
            pending_asks: Arc::new(Mutex::new(HashMap::new())),
            session_cwds: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
        };

        Ok(Env::new(skills, mcp_handler, cwd, memory, bridge))
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
    fn load_agents(
        runtime: &mut Runtime<ProviderRegistry, DaemonEnv>,
        config: &DaemonConfig,
        manifest: &ResolvedManifest,
    ) -> Result<()> {
        let prompts = crate::config::load_agents_dirs(&manifest.agent_dirs)?;
        let prompt_map: BTreeMap<String, String> = prompts.into_iter().collect();

        // Built-in crab agent.
        let mut crab_config = config.system.crab.clone();
        crab_config.name = wcore::paths::DEFAULT_AGENT.to_owned();
        crab_config.system_prompt = SYSTEM_AGENT.to_owned();
        runtime.add_agent(crab_config);

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
            resolve_package_skills(&mut agent.skills, &manifest.package_skill_dirs);
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
}

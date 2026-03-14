//! Daemon construction and lifecycle methods.
//!
//! This module provides the [`Daemon`] builder and reload logic as private
//! `impl Daemon` methods. [`Daemon::build`] constructs a fully-configured
//! daemon from a [`DaemonConfig`]. [`Daemon::reload`] rebuilds the runtime
//! in-place from disk without restarting transports.

use crate::{
    Daemon, DaemonConfig,
    daemon::event::{DaemonEvent, DaemonEventSender},
    ext::hub::DownloadRegistry,
    hook::{self, DaemonHook, task::TaskRegistry},
};
use anyhow::Result;
use compact_str::CompactString;
use model::ProviderManager;
use std::{path::Path, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use wcore::{AgentConfig, Runtime, ToolRequest};

const SYSTEM_AGENT: &str = include_str!("../../prompts/walrus.md");

impl Daemon {
    /// Build a fully-configured [`Daemon`] from the given config, config
    /// directory, and event sender.
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: DaemonEventSender,
    ) -> Result<Self> {
        let runtime = Self::build_runtime(config, config_dir, &event_tx).await?;
        Ok(Self {
            runtime: Arc::new(RwLock::new(Arc::new(runtime))),
            config_dir: config_dir.to_path_buf(),
            event_tx,
            agents_config: config.agents.clone(),
        })
    }

    /// Rebuild the runtime from disk and swap it in atomically.
    ///
    /// In-flight requests that already hold a reference to the old runtime
    /// complete normally. New requests after the swap see the new runtime.
    pub async fn reload(&self) -> Result<()> {
        let config = DaemonConfig::load(&self.config_dir.join("walrus.toml"))?;
        let new_runtime = Self::build_runtime(&config, &self.config_dir, &self.event_tx).await?;
        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("daemon reloaded");
        Ok(())
    }

    /// Construct a fresh [`Runtime`] from config. Used by both [`build`] and [`reload`].
    async fn build_runtime(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
    ) -> Result<Runtime<ProviderManager, DaemonHook>> {
        let manager = Self::build_providers(config).await?;
        let hook = Self::build_hook(config, config_dir, event_tx).await?;
        let tool_tx = Self::build_tool_sender(event_tx);
        let mut runtime = Runtime::new(manager, hook, Some(tool_tx)).await;
        Self::load_agents(&mut runtime, config_dir, config)?;
        Ok(runtime)
    }

    /// Construct the provider manager from config.
    ///
    /// Builds remote providers from config and sets the active model.
    async fn build_providers(config: &DaemonConfig) -> Result<ProviderManager> {
        let active_model = config
            .walrus
            .model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("walrus.model is required in walrus.toml"))?;
        let manager = ProviderManager::new(active_model.clone());

        // Add remote providers from config.
        for config in config.model.remotes.values() {
            manager.add_config(config).await?;
        }

        tracing::info!(
            "provider manager initialized — active model: {}",
            manager.active_model_name().unwrap_or_default()
        );
        Ok(manager)
    }

    /// Build the daemon hook with all backends (memory, skills, MCP, tasks, downloads).
    async fn build_hook(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
    ) -> Result<DaemonHook> {
        let downloads = Arc::new(Mutex::new(DownloadRegistry::new()));

        // Pre-download embeddings model files so MemoryHook::open() finds them cached.
        if let Err(e) = crate::ext::hub::embeddings::pre_download(&downloads).await {
            tracing::warn!("embeddings pre-download failed (memory may be degraded): {e}");
        }

        let memory_dir = config_dir.join("memory");
        let memory = hook::memory::MemoryHook::open(memory_dir, &config.memory).await?;
        tracing::info!("memory hook initialized (LanceDB graph)");

        let skills_dir = config_dir.join(wcore::paths::SKILLS_DIR);
        let skills = hook::skill::SkillHandler::load(skills_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load skills: {e}");
            hook::skill::SkillHandler::default()
        });

        let mcp_servers = config.mcps.values().cloned().collect::<Vec<_>>();
        let mcp_handler = hook::mcp::McpHandler::load(&mcp_servers).await;

        let tasks = Arc::new(Mutex::new(TaskRegistry::new(
            config.tasks.max_concurrent,
            config.tasks.viewable_window,
            std::time::Duration::from_secs(config.tasks.task_timeout),
            event_tx.clone(),
        )));

        let sandboxed = detect_sandbox();
        if sandboxed {
            tracing::info!("sandbox mode active — OS tools bypass permission check");
        }

        let aggregator = wsearch::aggregator::Aggregator::new(config.search.clone())
            .map_err(|e| anyhow::anyhow!("search init failed: {e}"))?;
        let fetch_client = wsearch::browser::fetch::default_client()
            .map_err(|e| anyhow::anyhow!("fetch client init failed: {e}"))?;
        tracing::info!("search tools initialized");

        Ok(DaemonHook::new(
            memory,
            skills,
            mcp_handler,
            tasks,
            downloads,
            config.permissions.clone(),
            sandboxed,
            aggregator,
            fetch_client,
        ))
    }

    /// Build a [`ToolSender`] that forwards [`ToolRequest`]s into the daemon
    /// event loop as [`DaemonEvent::ToolCall`] variants.
    ///
    /// Spawns a lightweight bridge task relaying from the tool channel into
    /// the main daemon event channel.
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
    ///
    /// The built-in walrus agent is always registered first. Sub-agents are
    /// loaded by iterating TOML `[agents.*]` entries and matching each to a
    /// `.md` prompt file from the agents directory.
    fn load_agents(
        runtime: &mut Runtime<ProviderManager, DaemonHook>,
        config_dir: &Path,
        config: &DaemonConfig,
    ) -> Result<()> {
        // Load prompt files from disk: (filename_stem, text).
        let prompts = crate::config::load_agents_dir(&config_dir.join(wcore::paths::AGENTS_DIR))?;
        let prompt_map: std::collections::BTreeMap<String, String> = prompts.into_iter().collect();

        // Built-in walrus agent.
        let mut walrus_config = config.walrus.clone();
        walrus_config.name = CompactString::from("walrus");
        walrus_config.system_prompt = SYSTEM_AGENT.to_owned();
        runtime.add_agent(walrus_config);

        // Sub-agents from TOML — each must have a matching .md file.
        for (name, agent_config) in &config.agents {
            let Some(prompt) = prompt_map.get(name) else {
                tracing::warn!("agent '{name}' in TOML has no matching .md file, skipping");
                continue;
            };
            let mut agent = agent_config.clone();
            agent.name = CompactString::from(name.as_str());
            agent.system_prompt = prompt.clone();
            tracing::info!("registered agent '{name}' (thinking={})", agent.thinking);
            runtime.add_agent(agent);
        }

        // Also register agents that have .md files but no TOML entry (defaults).
        let default_think = config.walrus.thinking;
        for (stem, prompt) in &prompt_map {
            if config.agents.contains_key(stem) {
                continue;
            }
            let mut agent = AgentConfig::new(stem.as_str());
            agent.system_prompt = prompt.clone();
            agent.thinking = default_think;
            tracing::info!("registered agent '{stem}' (defaults, thinking={default_think})");
            runtime.add_agent(agent);
        }

        // Populate per-agent scope maps for dispatch enforcement.
        for agent_config in runtime.agents() {
            runtime
                .hook
                .register_scope(agent_config.name.clone(), &agent_config);
        }

        Ok(())
    }
}

/// Detect sandbox mode by checking if the current process is running as
/// a user named `walrus`.
fn detect_sandbox() -> bool {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .is_ok_and(|u| u == "walrus")
}

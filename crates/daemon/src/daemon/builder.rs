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
    hook::{
        self, DaemonHook,
        system::{memory::BuiltinMemory, task::TaskRegistry},
    },
    service::ServiceManager,
};
use anyhow::Result;
use compact_str::CompactString;
use model::ProviderRegistry;
use std::{path::Path, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use wcore::{AgentConfig, Runtime, ToolRequest};

const SYSTEM_AGENT: &str = include_str!("../../prompts/walrus.md");
const SKILL_MASTER_AGENT: &str = include_str!("../../prompts/skill-master.md");

impl Daemon {
    /// Build a fully-configured [`Daemon`] from the given config, config
    /// directory, and event sender. Returns the daemon and an optional
    /// ServiceManager for lifecycle management of child services.
    pub(crate) async fn build(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: DaemonEventSender,
    ) -> Result<(Self, Option<ServiceManager>)> {
        let (runtime, service_manager) = Self::build_runtime(config, config_dir, &event_tx).await?;
        Ok((
            Self {
                runtime: Arc::new(RwLock::new(Arc::new(runtime))),
                config_dir: config_dir.to_path_buf(),
                event_tx,
                agents_config: config.agents.clone(),
            },
            service_manager,
        ))
    }

    /// Rebuild the runtime from disk and swap it in atomically.
    ///
    /// In-flight requests that already hold a reference to the old runtime
    /// complete normally. New requests after the swap see the new runtime.
    /// Note: reload does not restart managed services — that requires a
    /// full daemon restart. Services field is cleared to avoid re-spawning.
    pub async fn reload(&self) -> Result<()> {
        let mut config = DaemonConfig::load(&self.config_dir.join("walrus.toml"))?;
        config.services.clear();
        let (new_runtime, _) =
            Self::build_runtime(&config, &self.config_dir, &self.event_tx).await?;
        *self.runtime.write().await = Arc::new(new_runtime);
        tracing::info!("daemon reloaded");
        Ok(())
    }

    /// Construct a fresh [`Runtime`] from config. Used by both [`build`] and [`reload`].
    /// Returns the runtime and an optional ServiceManager for child service lifecycle.
    async fn build_runtime(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
    ) -> Result<(
        Runtime<ProviderRegistry, DaemonHook>,
        Option<ServiceManager>,
    )> {
        let manager = Self::build_providers(config)?;
        let (hook, service_manager) =
            Self::build_hook(config, config_dir, event_tx, &manager).await?;
        let tool_tx = Self::build_tool_sender(event_tx);
        let mut runtime = Runtime::new(manager, hook, Some(tool_tx)).await;
        // Set compact hook on runtime for auto-compaction.
        if let Some(ref registry) = runtime.hook.registry {
            runtime.set_compact_hook(Arc::clone(registry) as Arc<dyn wcore::CompactHook>);
        }
        Self::load_agents(&mut runtime, config_dir, config)?;
        Ok((runtime, service_manager))
    }

    /// Construct the provider registry from config.
    ///
    /// Builds remote providers from config and sets the active model.
    fn build_providers(config: &DaemonConfig) -> Result<ProviderRegistry> {
        let active_model = config
            .system
            .walrus
            .model
            .clone()
            .ok_or_else(|| anyhow::anyhow!("system.walrus.model is required in walrus.toml"))?;
        let registry = ProviderRegistry::from_providers(active_model, &config.provider)?;

        tracing::info!(
            "provider registry initialized — active model: {}",
            registry.active_model_name().unwrap_or_default()
        );
        Ok(registry)
    }

    /// Build the daemon hook with all backends (skills, MCP, tasks, downloads, memory).
    /// Built-in memory is active unless the walrus-memory extension provides `recall`.
    /// Returns the hook and an optional ServiceManager for child service lifecycle.
    async fn build_hook(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
        manager: &ProviderRegistry,
    ) -> Result<(DaemonHook, Option<ServiceManager>)> {
        let downloads = Arc::new(Mutex::new(DownloadRegistry::new()));

        let skills_dir = config_dir.join(wcore::paths::SKILLS_DIR);
        let skills = hook::skill::SkillHandler::load(skills_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load skills: {e}");
            hook::skill::SkillHandler::default()
        });

        let mcp_servers = config.mcps.values().cloned().collect::<Vec<_>>();
        let mcp_handler = hook::mcp::McpHandler::load(&mcp_servers).await;

        let tasks = Arc::new(Mutex::new(TaskRegistry::new(
            config.system.tasks.max_concurrent,
            config.system.tasks.viewable_window,
            std::time::Duration::from_secs(config.system.tasks.task_timeout),
            event_tx.clone(),
        )));

        let sandboxed = detect_sandbox();
        if sandboxed {
            tracing::info!("sandbox mode active — OS tools bypass permission check");
        }

        // Spawn and handshake managed services.
        let (registry, service_manager) = if config.services.is_empty() {
            (None, None)
        } else {
            let daemon_socket = wcore::paths::SOCKET_PATH.to_path_buf();
            let mut sm = ServiceManager::new(&config.services, config_dir, daemon_socket);
            sm.spawn_all().await?;
            let mut registry = sm.handshake_all().await;
            // Set model for Infer fulfillment before wrapping in Arc.
            registry.set_model(manager.clone());
            (Some(Arc::new(registry)), Some(sm))
        };

        // Construct built-in memory unless the walrus-memory extension provides "recall".
        let has_ext_memory = registry
            .as_ref()
            .is_some_and(|r| r.tools.contains_key("recall"));
        let memory = if !has_ext_memory {
            Some(BuiltinMemory::open(
                config_dir.join("memory"),
                config.system.memory.clone(),
            ))
        } else {
            None
        };

        Ok((
            DaemonHook::new(
                skills,
                mcp_handler,
                tasks,
                downloads,
                config.permissions.clone(),
                sandboxed,
                memory,
                registry,
            ),
            service_manager,
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
        runtime: &mut Runtime<ProviderRegistry, DaemonHook>,
        config_dir: &Path,
        config: &DaemonConfig,
    ) -> Result<()> {
        // Load prompt files from disk: (filename_stem, text).
        let prompts = crate::config::load_agents_dir(&config_dir.join(wcore::paths::AGENTS_DIR))?;
        let prompt_map: std::collections::BTreeMap<String, String> = prompts.into_iter().collect();

        // Built-in walrus agent.
        let mut walrus_config = config.system.walrus.clone();
        walrus_config.name = CompactString::from(wcore::paths::DEFAULT_AGENT);
        walrus_config.system_prompt = SYSTEM_AGENT.to_owned();
        runtime.add_agent(walrus_config);

        // Built-in skill-master agent.
        let mut skill_master = AgentConfig::new("skill-master");
        skill_master.system_prompt = SKILL_MASTER_AGENT.to_owned();
        skill_master.description = CompactString::from("Interactive skill recorder");
        skill_master.thinking = config.system.walrus.thinking;
        runtime.add_agent(skill_master);

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
        let default_think = config.system.walrus.thinking;
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

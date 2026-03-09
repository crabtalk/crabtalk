//! Daemon construction and lifecycle methods.
//!
//! This module provides the [`Daemon`] builder and reload logic as private
//! `impl Daemon` methods. [`Daemon::build`] constructs a fully-configured
//! daemon from a [`DaemonConfig`]. [`Daemon::reload`] rebuilds the runtime
//! in-place from disk without restarting transports.

use crate::{
    DaemonConfig,
    daemon::event::{DaemonEvent, DaemonEventSender},
    hook::{self, DaemonHook, task::TaskRegistry},
};
use anyhow::Result;
use model::ProviderManager;
use std::{path::Path, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use wcore::{Runtime, ToolRequest};

use super::Daemon;

const SYSTEM_AGENT: &str = include_str!("../../prompts/system.md");

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
            heartbeat_prompt: config.heartbeat.prompt.clone(),
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
        Self::load_agents(&mut runtime, config_dir)?;
        Ok(runtime)
    }

    /// Construct the provider manager from config.
    ///
    /// Loads a single local model from the registry (if local feature enabled)
    /// and any remote providers from config. Only one local model is active
    /// at a time to avoid memory pressure.
    async fn build_providers(config: &DaemonConfig) -> Result<ProviderManager> {
        let manager = ProviderManager::new(config.model.default.clone());

        // Add the single default local model from the registry.
        #[cfg(feature = "local")]
        {
            if let Some(entry) = model::local::registry::find(&config.model.default) {
                let local = model::local::registry::build_local(entry);
                manager
                    .add_provider(config.model.default.clone(), model::Provider::Local(local))?;
            } else if let Some(entry) = model::local::registry::find_by_key(&config.model.default) {
                let local = model::local::registry::build_local(entry);
                manager
                    .add_provider(config.model.default.clone(), model::Provider::Local(local))?;
            }
        }

        // Add remote providers from config.
        for config in config.model.providers.values() {
            manager.add_config(config).await?;
        }

        tracing::info!(
            "provider manager initialized — active model: {}",
            manager.active_model_name().unwrap_or_default()
        );
        Ok(manager)
    }

    /// Build the daemon hook with all backends (memory, skills, MCP, tasks).
    async fn build_hook(
        config: &DaemonConfig,
        config_dir: &Path,
        event_tx: &DaemonEventSender,
    ) -> Result<DaemonHook> {
        let memory_dir = config_dir.join("memory");
        let memory = hook::memory::MemoryHook::open(memory_dir, &config.memory).await?;
        tracing::info!("memory hook initialized (LanceDB graph)");

        let skills_dir = config_dir.join(wcore::paths::SKILLS_DIR);
        let skills = hook::skill::SkillHandler::load(skills_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load skills: {e}");
            hook::skill::SkillHandler::default()
        });

        let mcp_servers = config.mcp_servers.values().cloned().collect::<Vec<_>>();
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

        Ok(DaemonHook::new(
            memory,
            skills,
            mcp_handler,
            tasks,
            config.permissions.clone(),
            sandboxed,
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

    /// Load agents from markdown files and add them to the runtime.
    fn load_agents(
        runtime: &mut Runtime<ProviderManager, DaemonHook>,
        config_dir: &Path,
    ) -> Result<()> {
        let agents = crate::config::load_agents_dir(&config_dir.join(wcore::paths::AGENTS_DIR))?;
        runtime.add_agent(wcore::parse_agent_md(SYSTEM_AGENT)?);
        for agent in agents {
            tracing::info!("registered agent '{}'", agent.name);
            runtime.add_agent(agent);
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

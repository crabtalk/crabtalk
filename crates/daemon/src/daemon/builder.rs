//! Daemon construction and lifecycle methods.
//!
//! This module provides the [`Daemon`] builder and reload logic as private
//! `impl Daemon` methods. [`Daemon::build`] constructs a fully-configured
//! daemon from a [`DaemonConfig`]. [`Daemon::reload`] rebuilds the runtime
//! in-place from disk without restarting transports.

use crate::{
    DaemonConfig, config,
    daemon::event::{DaemonEvent, DaemonEventSender},
    hook::{self, DaemonHook},
};
use anyhow::Result;
use model::ProviderManager;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;
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
        let hook = Self::build_hook(config, config_dir).await;
        let tool_tx = Self::build_tool_sender(event_tx);
        let mut runtime = Runtime::new(manager, hook, Some(tool_tx)).await;
        Self::load_agents(&mut runtime, config_dir)?;
        Ok(runtime)
    }

    /// Construct the provider manager from config.
    async fn build_providers(config: &DaemonConfig) -> Result<ProviderManager> {
        let models = config.models.values().cloned().collect::<Vec<_>>();
        let manager = ProviderManager::from_configs(&models).await?;
        tracing::info!(
            "provider manager initialized — active model: {}",
            manager.active_model()
        );
        Ok(manager)
    }

    /// Build the daemon hook with all backends (memory, skills, MCP).
    async fn build_hook(config: &DaemonConfig, config_dir: &Path) -> DaemonHook {
        let memory = memory::InMemory::new();
        tracing::info!("using in-memory backend");

        let skills_dir = config_dir.join(config::SKILLS_DIR);
        let skills = hook::skill::SkillHandler::load(skills_dir).unwrap_or_else(|e| {
            tracing::warn!("failed to load skills: {e}");
            hook::skill::SkillHandler::load(PathBuf::new()).expect("empty skill handler")
        });

        let mcp_servers = config.mcp_servers.values().cloned().collect::<Vec<_>>();
        let mcp_handler = hook::mcp::McpHandler::load(&mcp_servers).await;

        DaemonHook::new(memory, skills, mcp_handler)
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
        let agents = crate::config::load_agents_dir(&config_dir.join(config::AGENTS_DIR))?;
        runtime.add_agent(wcore::parse_agent_md(SYSTEM_AGENT)?);
        for agent in agents {
            tracing::info!("registered agent '{}'", agent.name);
            runtime.add_agent(agent);
        }
        Ok(())
    }
}

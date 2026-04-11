//! Node construction and lifecycle methods.

use crate::mcp::McpHandler;
use crate::{
    Node, NodeConfig,
    node::event::{NodeEvent, NodeEventSender},
    storage::FsStorage,
};
use anyhow::Result;
use crabllm_core::Provider;
use crabllm_provider::{ProviderRegistry, RemoteProvider};
use runtime::{Env, Memory, Runtime, host::Host};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, broadcast};
use wcore::{AgentConfig, ToolRequest, model::Model, repos::Storage};
use wcore::{ResolvedManifest, resolve_manifests};

pub type DefaultProvider = crate::provider::Retrying<ProviderRegistry<RemoteProvider>>;

pub type BuildProvider<P> =
    Arc<dyn Fn(&NodeConfig) -> Result<wcore::model::Model<P>> + Send + Sync>;

pub fn build_default_provider(config: &NodeConfig) -> Result<Model<DefaultProvider>> {
    build_providers(config)
}

pub(crate) const SYSTEM_AGENT: &str = runtime::memory::DEFAULT_SOUL;

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

impl<P: Provider + 'static, H: Host + 'static> Node<P, H> {
    pub(crate) async fn build(
        config: &NodeConfig,
        config_dir: &Path,
        event_tx: NodeEventSender,
        shutdown_tx: broadcast::Sender<()>,
        host: H,
        build_provider: BuildProvider<P>,
        pending_asks: Option<crate::tools::ask_user::PendingAsks>,
    ) -> Result<Self> {
        if let Err(e) = crate::storage::backfill_local_agent_ids(config_dir) {
            tracing::warn!("agent id backfill failed: {e}");
        }

        let runtime = Self::build_runtime(
            config,
            config_dir,
            &event_tx,
            host,
            &build_provider,
            pending_asks.clone(),
        )
        .await?;
        let cron_store =
            crate::cron::CronStore::load(config_dir.to_path_buf(), event_tx.clone(), shutdown_tx);
        let crons = Arc::new(Mutex::new(cron_store));
        crons.lock().await.start_all(crons.clone());

        let fire_tx = event_tx.clone();
        let fire: crate::event_bus::FireCallback = Arc::new(move |sub, payload| {
            use wcore::protocol::message::{ClientMessage, SendMsg};
            let (reply_tx, _) = tokio::sync::mpsc::channel(1);
            let msg = ClientMessage::from(SendMsg {
                agent: sub.target_agent.clone(),
                content: payload.to_owned(),
                sender: Some(format!("event:{}", sub.source)),
                cwd: None,
                guest: None,
                tool_choice: None,
            });
            let _ = fire_tx.send(NodeEvent::Message {
                msg,
                reply: reply_tx,
            });
        });
        let event_bus = crate::event_bus::EventBus::load(config_dir.to_path_buf(), fire);
        let events = Arc::new(Mutex::new(event_bus));
        Ok(Self {
            runtime: Arc::new(RwLock::new(Arc::new(runtime))),
            config_dir: config_dir.to_path_buf(),
            event_tx,
            started_at: std::time::Instant::now(),
            crons,
            events,
            build_provider,
            pending_asks,
        })
    }

    pub async fn reload(&self) -> Result<()> {
        let config = NodeConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))?;
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
            self.pending_asks.clone(),
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

    async fn build_runtime(
        config: &NodeConfig,
        config_dir: &Path,
        event_tx: &NodeEventSender,
        mut host: H,
        build_provider: &BuildProvider<P>,
        pending_asks: Option<crate::tools::ask_user::PendingAsks>,
    ) -> Result<Runtime<crate::node::NodeCfg<P, H>>> {
        let (mut manifest, _warnings) = resolve_manifests(config_dir);
        manifest.disabled = config.disabled.clone();
        wcore::filter_disabled_external(&mut manifest.skill_dirs, &manifest.disabled.external);
        let model = build_provider(config)?;

        // Build MCP before the env so the host has it from the start.
        let servers = mcp_servers(config, &manifest);
        let mcp_handler: Arc<McpHandler> = Arc::new(McpHandler::load(&servers).await);
        host.set_mcp(mcp_handler.clone());

        let (hook, storage, tools) = build_env(
            config,
            config_dir,
            &manifest,
            host,
            event_tx.clone(),
            mcp_handler,
            pending_asks,
        )?;
        let tool_tx = build_tool_sender(event_tx);
        let mut runtime = Runtime::new(model, hook, storage, Some(tool_tx), tools);
        load_agents(&mut runtime, config_dir, config, &manifest)?;
        Ok(runtime)
    }
}

fn build_providers(config: &NodeConfig) -> Result<Model<DefaultProvider>> {
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

/// Build MCP server configs from manifest + node config env vars.
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

fn build_env<H: Host + 'static>(
    config: &NodeConfig,
    config_dir: &Path,
    manifest: &ResolvedManifest,
    host: H,
    event_tx: NodeEventSender,
    mcp_handler: Arc<McpHandler>,
    pending_asks: Option<crate::tools::ask_user::PendingAsks>,
) -> Result<(Env<H, FsStorage>, Arc<FsStorage>, wcore::ToolRegistry)> {
    let skill_roots: Vec<PathBuf> = manifest
        .skill_dirs
        .iter()
        .filter(|dir| dir.exists())
        .cloned()
        .collect();
    let memory_root = config_dir.join("memory");
    let sessions_root = config_dir.join("sessions");

    let storage = Arc::new(FsStorage::new(
        config_dir.to_path_buf(),
        memory_root,
        sessions_root,
        skill_roots,
        manifest.disabled.skills.clone(),
        manifest.agent_dirs.clone(),
    ));

    let memory = Arc::new(Memory::open(config.system.memory.clone(), storage.clone()));
    let cwd = std::env::current_dir().unwrap_or_else(|_| config_dir.to_path_buf());
    let scopes = Arc::new(std::sync::RwLock::new(BTreeMap::new()));

    let mut env = Env::new(
        storage.clone(),
        cwd.clone(),
        Some(memory.clone()),
        host.clone(),
        scopes.clone(),
    );

    // Register tool handlers.
    let mut tools = wcore::ToolRegistry::new();

    let register = |tools: &mut wcore::ToolRegistry,
                    env: &mut Env<H, FsStorage>,
                    entry: (wcore::model::Tool, wcore::ToolHandler)| {
        let (schema, handler) = entry;
        let name = schema.function.name.clone();
        tools.insert(schema);
        env.register_handler(name, handler);
    };

    register(
        &mut tools,
        &mut env,
        crate::tools::os::bash(cwd.clone(), host.clone()),
    );
    register(
        &mut tools,
        &mut env,
        crate::tools::os::read(cwd.clone(), host.clone()),
    );
    register(
        &mut tools,
        &mut env,
        crate::tools::os::edit(cwd, host.clone()),
    );

    for entry in crate::tools::memory::handlers(memory) {
        register(&mut tools, &mut env, entry);
    }

    register(
        &mut tools,
        &mut env,
        crate::tools::skill::handler(storage.clone(), scopes.clone()),
    );
    register(
        &mut tools,
        &mut env,
        crate::tools::delegate::handler(event_tx, scopes.clone()),
    );

    if let Some(pending_asks) = pending_asks {
        register(
            &mut tools,
            &mut env,
            crate::tools::ask_user::handler(pending_asks),
        );
    }

    // MCP tools — each schema gets the same shared handler.
    let mcp_tools = host.mcp_tools();
    if !mcp_tools.is_empty() {
        let mcp_h = crate::tools::mcp::handler(mcp_handler, scopes.clone());
        for schema in mcp_tools {
            let name = schema.function.name.clone();
            tools.insert(schema);
            env.register_handler(name, mcp_h.clone());
        }
    }

    Ok((env, storage, tools))
}

fn build_tool_sender(event_tx: &NodeEventSender) -> wcore::ToolSender {
    let (tool_tx, mut tool_rx) = tokio::sync::mpsc::unbounded_channel::<ToolRequest>();
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(req) = tool_rx.recv().await {
            if event_tx.send(NodeEvent::ToolCall(req)).is_err() {
                break;
            }
        }
    });
    tool_tx
}

fn load_agents<P: Provider + 'static, H: Host + 'static>(
    runtime: &mut Runtime<crate::node::NodeCfg<P, H>>,
    config_dir: &Path,
    config: &NodeConfig,
    manifest: &ResolvedManifest,
) -> Result<()> {
    // One-shot migration: hoist legacy prompt files into ULID-keyed storage.
    if let Err(e) = crate::storage::migrate_local_agent_prompts(
        config_dir,
        manifest,
        runtime.storage().as_ref(),
    ) {
        tracing::warn!("local agent prompt migration failed: {e}");
    }

    let prompts = wcore::load_agents_dirs(&manifest.agent_dirs)?;
    let prompt_map: BTreeMap<String, String> = prompts.into_iter().collect();

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
    runtime.add_agent(crab_config);

    // Sub-agents from manifests.
    let storage = runtime.storage().clone();
    for (name, agent_config) in &manifest.agents {
        if name == wcore::paths::DEFAULT_AGENT {
            tracing::warn!(
                "agents.{name} overrides the built-in system agent and will be ignored — \
                 configure it under [system.crab] instead"
            );
            continue;
        }
        let Some(prompt) = resolve_agent_prompt(storage.as_ref(), agent_config, name, &prompt_map)
        else {
            tracing::warn!("agent '{name}' has no prompt, skipping");
            continue;
        };
        let mut agent = agent_config.clone();
        agent.name = name.clone();
        agent.system_prompt = prompt;
        if agent.model.is_none() {
            agent.model = Some(default_model.clone());
        }
        tracing::info!("registered agent '{name}' (thinking={})", agent.thinking);
        runtime.add_agent(agent);
    }

    // Agents with .md files but no manifest entry.
    let default_think = config.system.crab.thinking;
    for (stem, prompt) in &prompt_map {
        if stem == wcore::paths::DEFAULT_AGENT {
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

/// Resolve an agent's prompt, preferring the repo (ULID key) and falling
/// back to the legacy filesystem prompt map.
fn resolve_agent_prompt(
    storage: &impl Storage,
    config: &AgentConfig,
    name: &str,
    prompt_map: &BTreeMap<String, String>,
) -> Option<String> {
    if !config.id.is_nil()
        && let Ok(Some(loaded)) = storage.load_agent(&config.id)
        && !loaded.system_prompt.is_empty()
    {
        return Some(loaded.system_prompt);
    }
    prompt_map.get(name).cloned()
}

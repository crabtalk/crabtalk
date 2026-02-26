//! Walrus gateway binary entry point.
//!
//! Loads TOML configuration, constructs memory backend, provider, and
//! runtime, wires all subsystems, and runs the axum server with
//! graceful shutdown on ctrl-c.

use anyhow::Result;
use deepseek::DeepSeek;
use llm::LLM;
use runtime::{
    DEFAULT_COMPACT_PROMPT, DEFAULT_FLUSH_PROMPT, General, Hook, McpBridge, Runtime, SkillRegistry,
};
use std::sync::Arc;
use tokio::signal;
use tracing_subscriber::EnvFilter;
use walrus_gateway::{
    ApiKeyAuthenticator, AppState, Gateway, GatewayConfig, MemoryBackend, SessionManager,
    config::MemoryBackendKind,
};

/// Type-level hook wiring `MemoryBackend` as the memory implementation.
pub struct GatewayHook;

impl Hook for GatewayHook {
    type Provider = DeepSeek;
    type Memory = MemoryBackend;

    fn compact() -> &'static str {
        DEFAULT_COMPACT_PROMPT
    }

    fn flush() -> &'static str {
        DEFAULT_FLUSH_PROMPT
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing from RUST_LOG (default: info).
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Load configuration.
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gateway.toml".to_string());
    let config = GatewayConfig::load(&config_path)?;
    tracing::info!("loaded configuration from {config_path}");

    // Construct memory backend.
    let memory = match config.memory.backend {
        MemoryBackendKind::InMemory => {
            tracing::info!("using in-memory backend");
            MemoryBackend::in_memory()
        }
        MemoryBackendKind::Sqlite => {
            let path = config.memory.path.as_deref().unwrap_or("walrus-gateway.db");
            tracing::info!("using sqlite backend at {path}");
            MemoryBackend::sqlite(path)?
        }
    };

    // Construct provider.
    let provider = DeepSeek::new(llm::Client::new(), &config.llm.api_key)?;
    tracing::info!("provider initialized for model {}", config.llm.model);

    // Build general config.
    let general = General {
        model: config.llm.model.clone(),
        ..General::default()
    };

    // Build runtime.
    let mut runtime = Runtime::<GatewayHook>::new(general, provider, memory);

    // Load skills if configured.
    if let Some(ref skills_config) = config.skills {
        let registry =
            SkillRegistry::load_dir(&skills_config.directory, agent::SkillTier::Workspace)?;
        runtime.set_skills(registry);
        tracing::info!("loaded skills from {}", skills_config.directory);
    }

    // Connect MCP servers if configured.
    if !config.mcp_servers.is_empty() {
        let bridge = McpBridge::new();
        for server_config in &config.mcp_servers {
            let mut cmd = tokio::process::Command::new(&server_config.command);
            cmd.args(&server_config.args);
            for (k, v) in &server_config.env {
                cmd.env(k, v);
            }
            if let Err(e) = bridge.connect_stdio(cmd).await {
                tracing::warn!("failed to connect MCP server '{}': {e}", server_config.name);
            } else {
                tracing::info!("connected MCP server '{}'", server_config.name);
            }
        }
        runtime.connect_mcp(bridge);
        if let Err(e) = runtime.register_mcp_tools().await {
            tracing::warn!("failed to register MCP tools: {e}");
        }
    }

    // Register agents from config.
    for agent_config in &config.agents {
        let agent = agent::Agent {
            name: agent_config.name.clone(),
            description: agent_config.description.clone(),
            system_prompt: agent_config.system_prompt.clone(),
            tools: agent_config.tools.clone(),
            skill_tags: agent_config.skill_tags.clone(),
        };
        tracing::info!("registered agent '{}'", agent.name);
        runtime.add_agent(agent);
    }

    // Build the gateway.
    let gateway = Gateway::new(config, runtime);
    let bind_address = gateway.config.bind_address();

    // Initialize authenticator from config api keys.
    let authenticator = ApiKeyAuthenticator::from_config(&gateway.config.auth);

    // Build app state.
    let state = AppState {
        runtime: Arc::clone(&gateway.runtime),
        sessions: Arc::new(SessionManager::new()),
        authenticator: Arc::new(authenticator),
    };

    // Build axum router.
    let app = walrus_gateway::ws::router(state);

    // Bind and serve.
    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    tracing::info!("gateway listening on {bind_address}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("gateway shut down");
    Ok(())
}

/// Wait for ctrl-c signal for graceful shutdown.
async fn shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
    tracing::info!("received shutdown signal");
}

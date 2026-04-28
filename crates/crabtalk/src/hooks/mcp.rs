//! MCP tool — as a Hook implementation.

use mcp::{McpHandler, dispatch::dispatch_mcp};
use parking_lot::RwLock;
use runtime::Hook;
use schemars::JsonSchema;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool};

/// Call an MCP tool by name, or list available tools if no exact match.
#[derive(Deserialize, JsonSchema)]
pub struct Mcp {
    /// Tool name to call. If no exact match, returns fuzzy matches.
    /// Leave empty to list all available MCP tools.
    pub name: String,
    /// JSON-encoded arguments string (only used when calling a tool).
    #[serde(default)]
    pub args: Option<String>,
}

/// MCP subsystem: routes tool calls to MCP servers per agent.
///
/// Agents declare their MCP configs inline (RFC 0193). This hook wires
/// the agent lifecycle (`on_register_agent` / `on_unregister_agent`) to
/// the daemon's `McpHandler` so each agent's declared MCPs are spawned
/// or reconnected when the agent appears, and dispatch is scoped to
/// only the MCPs the calling agent declared.
pub struct McpHook {
    mcp: Arc<McpHandler>,
    /// Daemon-wide env overlay applied on top of each MCP's own env at
    /// register time. Mirrors the pre-RFC behavior where the daemon's
    /// `[env]` section seeded variables for every MCP child process.
    env_overlay: BTreeMap<String, String>,
    /// Per-agent declared MCP server names. Populated from
    /// `on_register_agent`; consulted on dispatch to scope tool routing
    /// and on unregister for future cleanup.
    agent_mcps: RwLock<BTreeMap<String, Vec<String>>>,
}

impl McpHook {
    pub fn new(mcp: Arc<McpHandler>, env_overlay: BTreeMap<String, String>) -> Self {
        Self {
            mcp,
            env_overlay,
            agent_mcps: RwLock::new(BTreeMap::new()),
        }
    }
}

impl Hook for McpHook {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![Mcp::as_tool()]
    }

    fn scoped_tools(&self, config: &wcore::AgentConfig) -> (Vec<String>, Option<String>) {
        if config.mcps.is_empty() {
            return (vec![], None);
        }
        let tools = self
            .schema()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        let names: Vec<&str> = config.mcps.iter().map(|m| m.name.as_str()).collect();
        let line = format!("mcp servers: {}", names.join(", "));
        (tools, Some(line))
    }

    fn on_register_agent(&self, name: &str, config: &wcore::AgentConfig) {
        let mut names = Vec::with_capacity(config.mcps.len());
        for cfg in &config.mcps {
            names.push(cfg.name.clone());
            let mut effective = cfg.clone();
            for (k, v) in &self.env_overlay {
                effective.env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            // on_register_agent is sync; the connect itself is async.
            // Lifecycle events (Connecting/Connected/Failed) surface
            // outcome to subscribers — see RFC 0190.
            let handler = self.mcp.clone();
            tokio::spawn(async move {
                handler.upsert_server(&effective).await;
            });
        }
        self.agent_mcps.write().insert(name.to_owned(), names);
    }

    fn on_unregister_agent(&self, name: &str) {
        // Refcounted teardown lands with fingerprint-keyed dedup
        // (Phase 3). For now, agent removal forgets the bookkeeping
        // entry but leaves bridge peers connected — matches pre-refactor
        // behavior where named MCPs persisted independently of agents.
        self.agent_mcps.write().remove(name);
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if name != "mcp" {
            return None;
        }
        Some(Box::pin(async move {
            let allowed_mcps = self
                .agent_mcps
                .read()
                .get(&call.agent)
                .cloned()
                .unwrap_or_default();
            dispatch_mcp(&self.mcp, &call.args, &allowed_mcps).await
        }))
    }
}

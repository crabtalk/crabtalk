//! MCP tool — as a Harness implementation.

use mcp::{McpHandler, dispatch::dispatch_mcp};
use parking_lot::RwLock;
use runtime::{Harness, ToolDispatch, ToolFuture, agent::AsTool};
use schemars::JsonSchema;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};
use store::AgentId;

/// MCP subsystem: routes tool calls to MCP servers per agent.
///
/// Agents declare their MCP configs inline (RFC 0193). This hook wires
/// the agent lifecycle (`on_register_agent` / `on_unregister_agent`) to
/// the daemon's `McpHandler` so each agent's declared MCPs are spawned
/// or reconnected when the agent appears, and dispatch is scoped to the
/// MCPs the calling agent owns. Identical configs across agents share
/// one peer process — see `McpHandler::register_for_agent`.
pub struct McpHarness {
    pub handler: Arc<McpHandler>,
    /// Daemon-wide env overlay applied on top of each MCP's own env at
    /// register time.
    env_overlay: BTreeMap<String, String>,
    /// Per-agent declared MCP names. Snapshot of what's currently
    /// registered; consulted to scope dispatch and to compute the set
    /// of MCPs to unregister when the agent's declarations change.
    agent_mcps: RwLock<BTreeMap<AgentId, Vec<String>>>,
}

impl McpHarness {
    pub fn new(handler: Arc<McpHandler>, env_overlay: BTreeMap<String, String>) -> Self {
        Self {
            handler,
            env_overlay,
            agent_mcps: RwLock::new(BTreeMap::new()),
        }
    }
}

impl Harness for McpHarness {
    fn schema(&self) -> Vec<crabllm_core::Tool> {
        vec![Mcp::as_tool()]
    }

    fn scoped_tools(&self, config: &store::AgentConfig) -> (Vec<String>, Option<String>) {
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

    fn on_resolve_agent(&self, id: &AgentId, config: &store::AgentConfig) {
        let new_names: Vec<String> = config.mcps.iter().map(|m| m.name.clone()).collect();
        let prior = self
            .agent_mcps
            .write()
            .insert(*id, new_names.clone())
            .unwrap_or_default();

        // Drop any MCPs that disappeared (e.g., agent updated with
        // some entries removed). The handler handles refcounting —
        // a peer shared with another agent won't actually be torn down.
        for old_name in prior {
            if !new_names.contains(&old_name) {
                let handler = self.handler.clone();
                let agent = id.to_string();
                tokio::spawn(async move {
                    handler.unregister_for_agent(&agent, &old_name).await;
                });
            }
        }

        // Register every current declaration. Idempotent — a repeat
        // with the same fingerprint is a no-op refcount bump.
        for cfg in &config.mcps {
            let mut effective = cfg.clone();
            for (k, v) in &self.env_overlay {
                effective.env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            let handler = self.handler.clone();
            let agent = id.to_string();
            tokio::spawn(async move {
                handler.register_for_agent(&agent, &effective).await;
            });
        }
    }

    fn on_forget_agent(&self, id: &AgentId) {
        let Some(names) = self.agent_mcps.write().remove(id) else {
            return;
        };
        for mcp_name in names {
            let handler = self.handler.clone();
            let agent = id.to_string();
            tokio::spawn(async move {
                handler.unregister_for_agent(&agent, &mcp_name).await;
            });
        }
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
            // `lib/mcp` knows nothing about agents beyond an opaque scope
            // key, so identity crosses that seam as the ULID's text.
            dispatch_mcp(
                &self.handler,
                &call.agent.to_string(),
                &call.args,
                &allowed_mcps,
            )
            .await
        }))
    }
}

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

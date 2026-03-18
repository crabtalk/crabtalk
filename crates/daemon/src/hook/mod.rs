//! Stateful Hook implementation for the daemon.
//!
//! [`DaemonHook`] composes skill, MCP, OS, and built-in memory sub-hooks plus
//! external extension services. `on_build_agent` delegates to skills, memory,
//! and extension services; `on_register_tools` delegates to all sub-hooks in
//! sequence. `dispatch_tool` routes every agent tool call by name — the single
//! entry point from `event.rs`.

use crate::{
    daemon::event::DaemonEventSender,
    ext::hub::DownloadRegistry,
    hook::{
        mcp::McpHandler,
        os::PermissionConfig,
        skill::SkillHandler,
        system::{memory::Memory, task::TaskSet},
    },
    service::ServiceRegistry,
};
use compact_str::CompactString;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;
use wcore::{AgentConfig, AgentEvent, Hook, ToolRegistry, model::Message};

pub mod mcp;
pub mod os;
pub mod skill;
pub mod system;

/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub(crate) struct AgentScope {
    pub(crate) tools: Vec<CompactString>,
    pub(crate) members: Vec<String>,
    pub(crate) skills: Vec<String>,
    pub(crate) mcps: Vec<String>,
}

pub struct DaemonHook {
    pub skills: SkillHandler,
    pub mcp: McpHandler,
    pub tasks: Arc<Mutex<TaskSet>>,
    pub downloads: Arc<Mutex<DownloadRegistry>>,
    pub permissions: PermissionConfig,
    /// Whether the daemon is running as the `walrus` OS user (sandbox active).
    pub sandboxed: bool,
    /// Built-in memory.
    pub memory: Option<Memory>,
    /// Event channel for task dispatch.
    pub(crate) event_tx: DaemonEventSender,
    /// Per-agent scope maps, populated during load_agents.
    pub(crate) scopes: BTreeMap<CompactString, AgentScope>,
    /// Sub-agent descriptions for catalog injection into the walrus agent.
    pub(crate) agent_descriptions: BTreeMap<CompactString, CompactString>,
    /// External extension service registry (tools + queries).
    pub(crate) registry: Option<Arc<ServiceRegistry>>,
}

/// Base tools always included in every agent's whitelist.
/// Also bypass permission check when running in sandbox mode.
const BASE_TOOLS: &[&str] = &["read", "write", "edit", "bash"];

/// Skill discovery/loading tools.
const SKILL_TOOLS: &[&str] = &["search_skill", "load_skill", "save_skill"];

/// MCP discovery/call tools.
const MCP_TOOLS: &[&str] = &["search_mcp", "call_mcp_tool"];

/// Memory tools.
const MEMORY_TOOLS: &[&str] = &["recall", "remember", "memory", "forget", "soul"];

/// Task delegation tools.
const TASK_TOOLS: &[&str] = &["delegate", "collect", "check_tasks"];

impl Hook for DaemonHook {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        // Inject environment context (OS, working directory, sandbox state).
        config
            .system_prompt
            .push_str(&os::environment_block(self.sandboxed));

        // Inject built-in memory prompt if active.
        if let Some(ref mem) = self.memory {
            let prompt = mem.build_prompt();
            if !prompt.is_empty() {
                config.system_prompt.push_str(&prompt);
            }
        }

        // Apply scoped tool whitelist + prompt for sub-agents.
        self.apply_scope(&mut config);
        config
    }

    fn on_before_run(
        &self,
        agent: &str,
        history: &[wcore::model::Message],
    ) -> Vec<wcore::model::Message> {
        let mut messages = Vec::new();
        // Any agent with members gets the sub-agent catalog.
        let has_members = self
            .scopes
            .get(agent)
            .is_some_and(|s| !s.members.is_empty());
        if has_members && !self.agent_descriptions.is_empty() {
            let mut block = String::from("<agents>\n");
            for (name, desc) in &self.agent_descriptions {
                block.push_str(&format!("- {name}: {desc}\n"));
            }
            block.push_str("</agents>");
            messages.push(Message::user(block));
        }
        if let Some(ref mem) = self.memory {
            messages.extend(mem.before_run(history));
        }
        messages
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.mcp.register_tools(tools).await;
        tools.insert_all(os::tool::tools());
        tools.insert_all(skill::tool::tools());
        tools.insert_all(system::task::tool::tools());
        if let Some(ref registry) = self.registry {
            registry.register_tools(tools).await;
        }
        if self.memory.is_some() {
            tools.insert_all(system::memory::tool::tools());
        }
    }

    fn on_after_compact(&self, agent: &str, summary: &str) {
        if let Some(ref mem) = self.memory {
            mem.after_compact(agent, summary);
        }
    }

    fn on_event(&self, agent: &str, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent text delta");
            }
            AgentEvent::ThinkingDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent thinking delta");
            }
            AgentEvent::ToolCallsStart(calls) => {
                tracing::debug!(%agent, count = calls.len(), "agent tool calls started");
            }
            AgentEvent::ToolResult { call_id, .. } => {
                tracing::debug!(%agent, %call_id, "agent tool result");
            }
            AgentEvent::ToolCallsComplete => {
                tracing::debug!(%agent, "agent tool calls complete");
            }
            AgentEvent::Compact { summary } => {
                tracing::info!(%agent, summary_len = summary.len(), "context compacted");
                self.on_after_compact(agent, summary);
            }
            AgentEvent::Done(response) => {
                tracing::info!(
                    %agent,
                    iterations = response.iterations,
                    stop_reason = ?response.stop_reason,
                    "agent run complete"
                );
            }
        }
    }
}

impl DaemonHook {
    /// Create a new DaemonHook with the given backends.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        skills: SkillHandler,
        mcp: McpHandler,
        tasks: Arc<Mutex<TaskSet>>,
        downloads: Arc<Mutex<DownloadRegistry>>,
        permissions: PermissionConfig,
        sandboxed: bool,
        memory: Option<Memory>,
        registry: Option<Arc<ServiceRegistry>>,
        event_tx: DaemonEventSender,
    ) -> Self {
        Self {
            skills,
            mcp,
            tasks,
            downloads,
            permissions,
            sandboxed,
            memory,
            event_tx,
            scopes: BTreeMap::new(),
            agent_descriptions: BTreeMap::new(),
            registry,
        }
    }

    /// Register an agent's scope for dispatch enforcement.
    pub(crate) fn register_scope(&mut self, name: CompactString, config: &AgentConfig) {
        if name != wcore::paths::DEFAULT_AGENT && !config.description.is_empty() {
            self.agent_descriptions
                .insert(name.clone(), config.description.clone());
        }
        self.scopes.insert(
            name,
            AgentScope {
                tools: config.tools.clone(),
                members: config.members.clone(),
                skills: config.skills.clone(),
                mcps: config.mcps.clone(),
            },
        );
    }

    /// Apply scoped tool whitelist and scope prompt for sub-agents.
    /// No-op for the walrus agent (empty scoping = all tools).
    fn apply_scope(&self, config: &mut AgentConfig) {
        let has_scoping =
            !config.skills.is_empty() || !config.mcps.is_empty() || !config.members.is_empty();
        if !has_scoping {
            return;
        }

        // Base tools + memory + external service tools always included.
        let mut whitelist: Vec<CompactString> =
            BASE_TOOLS.iter().map(|&s| CompactString::from(s)).collect();
        if self.memory.is_some() {
            for &t in MEMORY_TOOLS {
                whitelist.push(CompactString::from(t));
            }
        }
        if let Some(ref registry) = self.registry {
            for tool_name in registry.tools.keys() {
                whitelist.push(CompactString::from(tool_name.as_str()));
            }
        }
        let mut scope_lines = Vec::new();

        if !config.skills.is_empty() {
            for &t in SKILL_TOOLS {
                whitelist.push(CompactString::from(t));
            }
            scope_lines.push(format!("skills: {}", config.skills.join(", ")));
        }

        if !config.mcps.is_empty() {
            for &t in MCP_TOOLS {
                whitelist.push(CompactString::from(t));
            }
            let mcp_servers = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(self.mcp.list())
            });
            let mut mcp_info = Vec::new();
            for (server_name, tool_names) in &mcp_servers {
                if config.mcps.iter().any(|m| m == server_name.as_str()) {
                    for tn in tool_names {
                        whitelist.push(tn.clone());
                    }
                    mcp_info.push(format!(
                        "  - {}: {}",
                        server_name,
                        tool_names
                            .iter()
                            .map(|t| t.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if !mcp_info.is_empty() {
                scope_lines.push(format!("mcp servers:\n{}", mcp_info.join("\n")));
            }
        }

        if !config.members.is_empty() {
            for &t in TASK_TOOLS {
                whitelist.push(CompactString::from(t));
            }
            scope_lines.push(format!("members: {}", config.members.join(", ")));
        }

        if !scope_lines.is_empty() {
            let scope_block = format!("\n\n<scope>\n{}\n</scope>", scope_lines.join("\n"));
            config.system_prompt.push_str(&scope_block);
        }

        config.tools = whitelist;
    }

    /// Check tool permission. Returns `Some(denied_message)` if denied,
    /// `None` if allowed.
    fn check_perm(&self, name: &str, agent: &str) -> Option<String> {
        // OS tools bypass permission when running in sandbox mode.
        if self.sandboxed && BASE_TOOLS.contains(&name) {
            return None;
        }
        use crate::hook::os::ToolPermission;
        match self.permissions.resolve(agent, name) {
            ToolPermission::Deny => Some(format!("permission denied: {name}")),
            // Sub-agents are autonomous — Ask treated as Allow.
            ToolPermission::Ask | ToolPermission::Allow => None,
        }
    }

    /// Dispatch to an external extension service if the tool is registered.
    /// Returns `None` if the tool is not in the registry (fall through to in-process).
    async fn dispatch_external(&self, name: &str, args: &str, agent: &str) -> Option<String> {
        self.registry
            .as_ref()?
            .dispatch_tool(name, args, agent, None)
            .await
    }

    /// Route a tool call by name to the appropriate handler.
    ///
    /// This is the single dispatch entry point — `event.rs` calls this
    /// and never matches on tool names itself. Unrecognised names are
    /// forwarded to the MCP bridge after a warn-level log.
    pub async fn dispatch_tool(&self, name: &str, args: &str, agent: &str) -> String {
        if let Some(denied) = self.check_perm(name, agent) {
            return denied;
        }
        // Dispatch enforcement: reject tools not in the agent's whitelist.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.tools.is_empty()
            && !scope.tools.iter().any(|t| t.as_str() == name)
        {
            return format!("tool not available: {name}");
        }
        match name {
            "search_mcp" => self.dispatch_search_mcp(args, agent).await,
            "call_mcp_tool" => self.dispatch_call_mcp_tool(args, agent).await,
            "search_skill" => self.dispatch_search_skill(args, agent).await,
            "load_skill" => self.dispatch_load_skill(args, agent).await,
            "save_skill" => self.dispatch_save_skill(args).await,
            "read" => self.dispatch_read(args).await,
            "write" => self.dispatch_write(args).await,
            "edit" => self.dispatch_edit(args).await,
            "bash" => self.dispatch_bash(args).await,
            "delegate" => self.dispatch_delegate(args, agent).await,
            "collect" => self.dispatch_collect(args).await,
            "check_tasks" => self.dispatch_check_tasks(args).await,
            "recall" => self.dispatch_recall(args).await,
            "remember" => self.dispatch_remember(args).await,
            "memory" => self.dispatch_memory(args).await,
            "forget" => self.dispatch_forget(args).await,
            "soul" => self.dispatch_soul(args).await,
            // External extension services, then MCP bridge as final fallback.
            name => {
                if let Some(result) = self.dispatch_external(name, args, agent).await {
                    return result;
                }
                tracing::debug!(tool = name, "forwarding tool to MCP bridge");
                let bridge = self.mcp.bridge().await;
                bridge.call(name, args).await
            }
        }
    }
}

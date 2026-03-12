//! Stateful Hook implementation for the daemon.
//!
//! [`DaemonHook`] composes memory, skill, MCP, and OS sub-hooks.
//! `on_build_agent` delegates to skills and memory; `on_register_tools`
//! delegates to all sub-hooks in sequence. `dispatch_tool` routes every
//! agent tool call by name — the single entry point from `event.rs`.

use crate::{
    ext::hub::DownloadRegistry,
    hook::{
        mcp::McpHandler, memory::MemoryHook, os::PermissionConfig, skill::SkillHandler,
        task::TaskRegistry,
    },
};
use compact_str::CompactString;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;
use wcore::{AgentConfig, AgentEvent, Hook, ToolRegistry};

pub mod mcp;
pub mod memory;
pub mod os;
pub mod search;
pub mod skill;
pub mod task;

/// Stateful Hook implementation for the daemon.
///
/// Composes memory, skill, MCP, and OS sub-hooks. Each sub-hook
/// self-registers its tools via `on_register_tools`. All tool dispatch
/// is routed through `dispatch_tool`.
/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub(crate) struct AgentScope {
    pub(crate) tools: Vec<CompactString>,
    pub(crate) members: Vec<String>,
    pub(crate) skills: Vec<String>,
    pub(crate) mcps: Vec<String>,
}

pub struct DaemonHook {
    pub memory: MemoryHook,
    pub skills: SkillHandler,
    pub mcp: McpHandler,
    pub tasks: Arc<Mutex<TaskRegistry>>,
    pub downloads: Arc<Mutex<DownloadRegistry>>,
    pub permissions: PermissionConfig,
    /// Whether the daemon is running as the `walrus` OS user (sandbox active).
    pub sandboxed: bool,
    /// Per-agent scope maps, populated during load_agents.
    pub(crate) scopes: BTreeMap<CompactString, AgentScope>,
    pub(crate) aggregator: wsearch::aggregator::Aggregator,
    pub(crate) fetch_client: reqwest::Client,
}

/// OS tool names — bypass permission check when running in sandbox mode.
const OS_TOOLS: &[&str] = &["read", "write", "edit", "bash"];

/// Base tools always included in every agent's whitelist (memory + OS).
const BASE_TOOLS: &[&str] = &[
    "remember",
    "recall",
    "relate",
    "connections",
    "compact",
    "distill",
    "__journal__",
    "read",
    "write",
    "edit",
    "bash",
    "web_search",
    "web_fetch",
];

/// Skill discovery/loading tools.
const SKILL_TOOLS: &[&str] = &["search_skill", "load_skill"];

/// MCP discovery/call tools.
const MCP_TOOLS: &[&str] = &["search_mcp", "call_mcp_tool"];

/// Task delegation tools.
const TASK_TOOLS: &[&str] = &[
    "spawn_task",
    "check_tasks",
    "create_task",
    "ask_user",
    "await_tasks",
];

impl DaemonHook {
    /// Create a new DaemonHook with the given backends.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory: MemoryHook,
        skills: SkillHandler,
        mcp: McpHandler,
        tasks: Arc<Mutex<TaskRegistry>>,
        downloads: Arc<Mutex<DownloadRegistry>>,
        permissions: PermissionConfig,
        sandboxed: bool,
        aggregator: wsearch::aggregator::Aggregator,
        fetch_client: reqwest::Client,
    ) -> Self {
        Self {
            memory,
            skills,
            mcp,
            tasks,
            downloads,
            permissions,
            sandboxed,
            scopes: BTreeMap::new(),
            aggregator,
            fetch_client,
        }
    }

    /// Register an agent's scope for dispatch enforcement.
    pub(crate) fn register_scope(&mut self, name: CompactString, config: &AgentConfig) {
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

    /// Check tool permission. Returns `Some(denied_message)` if denied,
    /// `None` if allowed.
    async fn check_perm(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        task_id: Option<u64>,
    ) -> Option<String> {
        // OS tools bypass permission when running in sandbox mode.
        if self.sandboxed && OS_TOOLS.contains(&name) {
            return None;
        }
        use crate::hook::os::ToolPermission;
        match self.permissions.resolve(agent, name) {
            ToolPermission::Deny => Some(format!("permission denied: {name}")),
            ToolPermission::Ask => {
                if let Some(tid) = task_id {
                    let summary = if args.len() > 200 {
                        format!("{}…", &args[..200])
                    } else {
                        args.to_string()
                    };
                    let question = format!("{name}: {summary}");
                    let rx = self.tasks.lock().await.block(tid, question);
                    if let Some(rx) = rx {
                        match rx.await {
                            Ok(resp) if resp == "denied" => {
                                return Some(format!("permission denied: {name}"));
                            }
                            Err(_) => {
                                return Some(format!("permission denied: {name} (inbox dropped)"));
                            }
                            _ => {} // approved → proceed
                        }
                    }
                }
                // No task_id → can't block, treat as Allow.
                None
            }
            ToolPermission::Allow => None,
        }
    }

    /// Route a tool call by name to the appropriate handler.
    ///
    /// This is the single dispatch entry point — `event.rs` calls this
    /// and never matches on tool names itself. Unrecognised names are
    /// forwarded to the MCP bridge after a warn-level log.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        task_id: Option<u64>,
        sender: &str,
    ) -> String {
        if let Some(denied) = self.check_perm(name, args, agent, task_id).await {
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
            "remember" => self.memory.dispatch_remember(args, agent, sender).await,
            "recall" => self.memory.dispatch_recall(args, agent, sender).await,
            "relate" => self.memory.dispatch_relate(args, agent, sender).await,
            "connections" => self.memory.dispatch_connections(args, agent, sender).await,
            "compact" => self.memory.dispatch_compact(agent).await,
            "__journal__" => self.memory.dispatch_journal(args, agent).await,
            "distill" => self.memory.dispatch_distill(args, agent).await,
            "search_mcp" => self.dispatch_search_mcp(args, agent).await,
            "call_mcp_tool" => self.dispatch_call_mcp_tool(args, agent).await,
            "search_skill" => self.dispatch_search_skill(args, agent).await,
            "load_skill" => self.dispatch_load_skill(args, agent).await,
            "read" => self.dispatch_read(args).await,
            "write" => self.dispatch_write(args).await,
            "edit" => self.dispatch_edit(args).await,
            "bash" => self.dispatch_bash(args).await,
            "spawn_task" => self.dispatch_spawn_task(args, agent, task_id).await,
            "check_tasks" => self.dispatch_check_tasks(args).await,
            "create_task" => self.dispatch_create_task(args, agent).await,
            "ask_user" => self.dispatch_ask_user(args, task_id).await,
            "await_tasks" => self.dispatch_await_tasks(args, task_id).await,
            "web_search" => self.dispatch_web_search(args).await,
            "web_fetch" => self.dispatch_web_fetch(args).await,
            name => {
                tracing::debug!(tool = name, "forwarding tool to MCP bridge");
                let bridge = self.mcp.bridge().await;
                bridge.call(name, args).await
            }
        }
    }
}

impl Hook for DaemonHook {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        let mut config = self.memory.on_build_agent(config);

        // Walrus agent (empty scoping) gets all tools, no scope injection.
        let has_scoping =
            !config.skills.is_empty() || !config.mcps.is_empty() || !config.members.is_empty();
        if !has_scoping {
            return config;
        }

        // Compute tool whitelist — base tools always included.
        let mut whitelist: Vec<CompactString> =
            BASE_TOOLS.iter().map(|&s| CompactString::from(s)).collect();
        let mut scope_lines = Vec::new();

        // Skill tools if skills non-empty.
        if !config.skills.is_empty() {
            for &t in SKILL_TOOLS {
                whitelist.push(CompactString::from(t));
            }
            scope_lines.push(format!("skills: {}", config.skills.join(", ")));
        }

        // MCP tools if mcps non-empty.
        if !config.mcps.is_empty() {
            for &t in MCP_TOOLS {
                whitelist.push(CompactString::from(t));
            }
            // Also include tools from named MCP servers.
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

        // Task tools if members non-empty.
        if !config.members.is_empty() {
            for &t in TASK_TOOLS {
                whitelist.push(CompactString::from(t));
            }
            scope_lines.push(format!("members: {}", config.members.join(", ")));
        }

        // Inject scope info into system prompt.
        if !scope_lines.is_empty() {
            let scope_block = format!("\n\n<scope>\n{}\n</scope>", scope_lines.join("\n"));
            config.system_prompt.push_str(&scope_block);
        }

        config.tools = whitelist;
        config
    }

    fn on_compact(&self, prompt: &mut String) {
        self.memory.on_compact(prompt);
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.memory.on_register_tools(tools).await;
        self.mcp.on_register_tools(tools).await;
        tools.insert_all(os::tool::tools());
        tools.insert_all(search::tool::tools());
        tools.insert_all(skill::tool::tools());
        tools.insert_all(task::tool::tools());
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
            AgentEvent::Done(response) => {
                tracing::info!(
                    %agent,
                    iterations = response.iterations,
                    stop_reason = ?response.stop_reason,
                    "agent run complete"
                );
                // Track token usage on the active task for this agent.
                let (prompt, completion) = response.steps.iter().fold((0u64, 0u64), |(p, c), s| {
                    (
                        p + u64::from(s.response.usage.prompt_tokens),
                        c + u64::from(s.response.usage.completion_tokens),
                    )
                });
                if (prompt > 0 || completion > 0)
                    && let Ok(mut registry) = self.tasks.try_lock()
                {
                    let tid = registry
                        .list(Some(agent), Some(task::TaskStatus::InProgress), None)
                        .first()
                        .map(|t| t.id);
                    if let Some(tid) = tid {
                        registry.add_tokens(tid, prompt, completion);
                    }
                }
            }
        }
    }
}

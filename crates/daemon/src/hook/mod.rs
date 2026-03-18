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
    pub(crate) tools: Vec<String>,
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
    pub(crate) scopes: BTreeMap<String, AgentScope>,
    /// Sub-agent descriptions for catalog injection into the walrus agent.
    pub(crate) agent_descriptions: BTreeMap<String, String>,
    /// External extension service registry (tools + queries).
    pub(crate) registry: Option<Arc<ServiceRegistry>>,
}

/// Base tools always included in every agent's whitelist.
/// Also bypass permission check when running in sandbox mode.
const BASE_TOOLS: &[&str] = &["bash"];

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

        // Inject discoverable resource hints so the agent knows what's
        // available without resorting to bash exploration.
        let mut hints = Vec::new();
        let mcp_servers = self.mcp.cached_list();
        if !mcp_servers.is_empty() {
            let names: Vec<&str> = mcp_servers.iter().map(|(n, _)| n.as_str()).collect();
            hints.push(format!(
                "MCP servers: {}. Use search_mcp to list tools, call_mcp_tool to invoke them.",
                names.join(", ")
            ));
        }
        if let Ok(reg) = self.skills.registry.try_lock() {
            let skills: Vec<&str> = reg.skills().iter().map(|s| s.name.as_str()).collect();
            if !skills.is_empty() {
                hints.push(format!(
                    "Skills: {}. Use search_skill to find skills, load_skill to activate one.",
                    skills.join(", ")
                ));
            }
        }
        if !hints.is_empty() {
            config.system_prompt.push_str(&format!(
                "\n\n<resources>\n{}\n</resources>",
                hints.join("\n")
            ));
        }

        // Apply scoped tool whitelist + prompt for sub-agents.
        self.apply_scope(&mut config);
        config
    }

    fn preprocess(&self, agent: &str, content: &str) -> String {
        self.resolve_slash_skill(agent, content)
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
            let mut msg = Message::user(block);
            msg.auto_injected = true;
            messages.push(msg);
        }
        if let Some(ref mem) = self.memory {
            messages.extend(mem.before_run(history));
        }
        messages
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.mcp.register_tools(tools);
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
    pub(crate) fn register_scope(&mut self, name: String, config: &AgentConfig) {
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
        let mut whitelist: Vec<String> = BASE_TOOLS.iter().map(|&s| s.to_owned()).collect();
        if self.memory.is_some() {
            for &t in MEMORY_TOOLS {
                whitelist.push(t.to_owned());
            }
        }
        if let Some(ref registry) = self.registry {
            for tool_name in registry.tools.keys() {
                whitelist.push(tool_name.clone());
            }
        }
        let mut scope_lines = Vec::new();

        if !config.skills.is_empty() {
            for &t in SKILL_TOOLS {
                whitelist.push(t.to_owned());
            }
            scope_lines.push(format!("skills: {}", config.skills.join(", ")));
        }

        if !config.mcps.is_empty() {
            for &t in MCP_TOOLS {
                whitelist.push(t.to_owned());
            }
            let server_names: Vec<&str> = config.mcps.iter().map(|s| s.as_str()).collect();
            scope_lines.push(format!(
                "mcp servers: {}\nUse search_mcp to discover tools, call_mcp_tool to invoke them.",
                server_names.join(", ")
            ));
        }

        if !config.members.is_empty() {
            for &t in TASK_TOOLS {
                whitelist.push(t.to_owned());
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
    ///
    /// `Ask` permission: allowed for interactive sessions (sender is empty
    /// or "user"), denied for non-interactive (gateways, sub-agents).
    fn check_perm(&self, name: &str, agent: &str, sender: &str) -> Option<String> {
        // OS tools bypass permission when running in sandbox mode.
        if self.sandboxed && BASE_TOOLS.contains(&name) {
            return None;
        }
        use crate::hook::os::ToolPermission;
        match self.permissions.resolve(agent, name) {
            ToolPermission::Allow => None,
            ToolPermission::Deny => Some(format!("permission denied: {name}")),
            ToolPermission::Ask => {
                let interactive = sender.is_empty() || sender == "user";
                if interactive {
                    None
                } else {
                    tracing::warn!(
                        tool = name,
                        agent = agent,
                        sender = sender,
                        "tool requires approval — denied for non-interactive session"
                    );
                    Some(format!(
                        "permission denied: {name} (requires interactive approval)"
                    ))
                }
            }
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

    /// Resolve `/skill-name args` into skill body + args.
    /// Returns content unchanged if not a slash command or skill not found.
    fn resolve_slash_skill(&self, agent: &str, content: &str) -> String {
        let content = content.trim();
        if !content.starts_with('/') {
            return content.to_owned();
        }
        let rest = &content[1..];
        let (name, args) = match rest.find(' ') {
            Some(pos) => (&rest[..pos], Some(rest[pos + 1..].trim())),
            None => (rest, None),
        };
        // Guard against path traversal.
        if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
            return content.to_owned();
        }
        // Enforce skill scope.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.skills.is_empty()
            && !scope.skills.iter().any(|s| s == name)
        {
            return content.to_owned();
        }
        let skill_file = self.skills.skills_dir.join(name).join("SKILL.md");
        let Ok(file_content) = std::fs::read_to_string(&skill_file) else {
            return content.to_owned();
        };
        let Ok(skill) = skill::loader::parse_skill_md(&file_content) else {
            return content.to_owned();
        };
        match args {
            Some(a) if !a.is_empty() => format!("{}\n\n{a}", skill.body),
            _ => skill.body,
        }
    }

    /// Route a tool call by name to the appropriate handler.
    ///
    /// This is the single dispatch entry point — `event.rs` calls this
    /// and never matches on tool names itself. Unrecognised names are
    /// forwarded to the MCP bridge after a warn-level log.
    pub async fn dispatch_tool(&self, name: &str, args: &str, agent: &str, sender: &str) -> String {
        if let Some(denied) = self.check_perm(name, agent, sender) {
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
            "bash" => self.dispatch_bash(args).await,
            "delegate" => self.dispatch_delegate(args, agent).await,
            "collect" => self.dispatch_collect(args).await,
            "check_tasks" => self.dispatch_check_tasks(args).await,
            "recall" => self.dispatch_recall(args).await,
            "remember" => self.dispatch_remember(args).await,
            "memory" => self.dispatch_memory(args).await,
            "forget" => self.dispatch_forget(args).await,
            "soul" => self.dispatch_soul(args).await,
            // External extension services.
            name => {
                if let Some(result) = self.dispatch_external(name, args, agent).await {
                    return result;
                }
                format!("tool not available: {name}")
            }
        }
    }
}

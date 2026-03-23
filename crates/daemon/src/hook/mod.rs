//! Stateful Hook implementation for the daemon.
//!
//! [`DaemonHook`] composes skill, MCP, OS, and built-in memory sub-hooks.
//! `on_build_agent` delegates to skills, memory; `on_register_tools` delegates
//! to all sub-hooks in sequence. `dispatch_tool` routes every agent tool call
//! by name — the single entry point from `event.rs`.

use crate::{
    daemon::event::DaemonEventSender,
    hook::{mcp::McpHandler, skill::SkillHandler, system::memory::Memory},
};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{Mutex, broadcast, oneshot};
use wcore::{
    AgentConfig, AgentEvent, Hook, ToolRegistry,
    model::Message,
    protocol::message::{AgentEventKind, AgentEventMsg},
};

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
    /// Working directory for agent commands (caller's cwd at daemon startup).
    pub cwd: std::path::PathBuf,
    /// Built-in memory.
    pub memory: Option<Memory>,
    /// Event channel for task delegation.
    pub(crate) event_tx: DaemonEventSender,
    /// Per-agent scope maps, populated during load_agents.
    pub(crate) scopes: BTreeMap<String, AgentScope>,
    /// Sub-agent descriptions for catalog injection into the crab agent.
    pub(crate) agent_descriptions: BTreeMap<String, String>,
    /// Broadcast channel for agent events (console subscription).
    events_tx: broadcast::Sender<AgentEventMsg>,
    /// Pending `ask_user` oneshots, keyed by session_id.
    pub(crate) pending_asks: Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>,
    /// Per-session working directory overrides. Populated by protocol handler,
    /// used by `dispatch_bash` to resolve the caller's cwd.
    pub(crate) session_cwds: Arc<Mutex<HashMap<u64, PathBuf>>>,
}

/// Base tools always included in every agent's whitelist.
const BASE_TOOLS: &[&str] = &["bash", "ask_user"];

/// Skill discovery/loading tools.
const SKILL_TOOLS: &[&str] = &["skill"];

/// MCP discovery/call tools.
const MCP_TOOLS: &[&str] = &["mcp"];

/// Memory tools.
const MEMORY_TOOLS: &[&str] = &["recall", "remember", "memory", "forget"];

/// Task delegation tools.
const TASK_TOOLS: &[&str] = &["delegate"];

impl Hook for DaemonHook {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        // Inject environment context (OS info). Working directory is
        // injected per-session in on_before_run.
        config.system_prompt.push_str(&os::environment_block());

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
                "MCP servers: {}. Use the mcp tool to list or call tools.",
                names.join(", ")
            ));
        }
        if let Ok(reg) = self.skills.registry.try_lock() {
            let all_skills = reg.skills();
            // If the agent declares specific skills, show only those with
            // descriptions. Otherwise show all skills.
            let visible: Vec<_> = if config.skills.is_empty() {
                all_skills.iter().collect()
            } else {
                all_skills
                    .iter()
                    .filter(|s| config.skills.iter().any(|n| n == &s.name))
                    .collect()
            };
            if !visible.is_empty() {
                let lines: Vec<String> = visible
                    .iter()
                    .map(|s| {
                        if s.description.is_empty() {
                            format!("- {}", s.name)
                        } else {
                            format!("- {}: {}", s.name, s.description)
                        }
                    })
                    .collect();
                hints.push(format!(
                    "Skills (use the skill tool to load one by name):\n{}",
                    lines.join("\n")
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
        session_id: u64,
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
        // Inject per-session working directory.
        let cwd = self
            .session_cwds
            .try_lock()
            .ok()
            .and_then(|m| m.get(&session_id).cloned())
            .unwrap_or_else(|| self.cwd.clone());
        let mut cwd_msg = Message::user(format!(
            "<environment>\nworking_directory: {}\n</environment>",
            cwd.display()
        ));
        cwd_msg.auto_injected = true;
        messages.push(cwd_msg);
        messages
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.mcp.register_tools(tools);
        tools.insert_all(os::tool::tools());
        tools.insert_all(skill::tool::tools());
        tools.insert_all(system::task::tool::tools());
        tools.insert_all(system::ask_user::tools());
        if self.memory.is_some() {
            tools.insert_all(system::memory::tool::tools());
        }
    }

    fn on_after_compact(&self, agent: &str, summary: &str) {
        if let Some(ref mem) = self.memory {
            mem.after_compact(agent, summary);
        }
    }

    fn on_event(&self, agent: &str, session_id: u64, event: &AgentEvent) {
        let (kind, content) = match event {
            AgentEvent::TextDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent text delta");
                (AgentEventKind::TextDelta, String::new())
            }
            AgentEvent::ThinkingDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent thinking delta");
                (AgentEventKind::ThinkingDelta, String::new())
            }
            AgentEvent::ToolCallsStart(calls) => {
                tracing::debug!(%agent, count = calls.len(), "agent tool calls started");
                let names: Vec<&str> = calls.iter().map(|c| c.function.name.as_str()).collect();
                (AgentEventKind::ToolStart, names.join(", "))
            }
            AgentEvent::ToolResult { call_id, .. } => {
                tracing::debug!(%agent, %call_id, "agent tool result");
                (AgentEventKind::ToolResult, call_id.clone())
            }
            AgentEvent::ToolCallsComplete => {
                tracing::debug!(%agent, "agent tool calls complete");
                (AgentEventKind::ToolsComplete, String::new())
            }
            AgentEvent::Compact { summary } => {
                tracing::info!(%agent, summary_len = summary.len(), "context compacted");
                self.on_after_compact(agent, summary);
                return;
            }
            AgentEvent::Done(response) => {
                tracing::info!(
                    %agent,
                    iterations = response.iterations,
                    stop_reason = ?response.stop_reason,
                    "agent run complete"
                );
                (AgentEventKind::Done, String::new())
            }
        };
        let _ = self.events_tx.send(AgentEventMsg {
            agent: agent.to_string(),
            session: session_id,
            kind: kind.into(),
            content,
        });
    }
}

impl DaemonHook {
    /// Create a new DaemonHook with the given backends.
    pub fn new(
        skills: SkillHandler,
        mcp: McpHandler,
        cwd: std::path::PathBuf,
        memory: Option<Memory>,
        event_tx: DaemonEventSender,
    ) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self {
            skills,
            mcp,
            cwd,
            memory,
            event_tx,
            scopes: BTreeMap::new(),
            agent_descriptions: BTreeMap::new(),
            events_tx,
            pending_asks: Arc::new(Mutex::new(HashMap::new())),
            session_cwds: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Subscribe to agent events (for console event streaming).
    pub fn subscribe_events(&self) -> broadcast::Receiver<AgentEventMsg> {
        self.events_tx.subscribe()
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
    /// No-op for the crab agent (empty scoping = all tools).
    fn apply_scope(&self, config: &mut AgentConfig) {
        let has_scoping =
            !config.skills.is_empty() || !config.mcps.is_empty() || !config.members.is_empty();
        if !has_scoping {
            return;
        }

        // Base tools + memory always included.
        let mut whitelist: Vec<String> = BASE_TOOLS.iter().map(|&s| s.to_owned()).collect();
        if self.memory.is_some() {
            for &t in MEMORY_TOOLS {
                whitelist.push(t.to_owned());
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
            scope_lines.push(format!("mcp servers: {}", server_names.join(", ")));
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

    /// Scan content for `/skill-name` tokens, load each skill found, and
    /// append their bodies to the end of the message.
    /// Tokens that don't match a skill are left as-is.
    fn resolve_slash_skill(&self, agent: &str, content: &str) -> String {
        let scope = self.scopes.get(agent);
        let mut appended = Vec::new();
        let mut rest = content;

        while let Some(slash) = rest.find('/') {
            rest = &rest[slash + 1..];
            // Extract the skill name token: [a-z][a-z0-9-]*
            let end = rest
                .find(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            rest = &rest[end..];

            if name.is_empty() || name.contains("..") {
                continue;
            }
            // Enforce skill scope.
            if let Some(scope) = scope
                && !scope.skills.is_empty()
                && !scope.skills.iter().any(|s| s == name)
            {
                continue;
            }
            let mut found = false;
            for dir in &self.skills.skill_dirs {
                let skill_file = dir.join(name).join("SKILL.md");
                let Ok(file_content) = std::fs::read_to_string(&skill_file) else {
                    continue;
                };
                let Ok(skill) = skill::loader::parse_skill_md(&file_content) else {
                    continue;
                };
                appended.push(skill.body);
                found = true;
                break;
            }
            if !found {
                continue;
            }
        }

        if appended.is_empty() {
            return content.to_owned();
        }
        format!("{}\n\n{}", content, appended.join("\n\n"))
    }

    /// Route a tool call by name to the appropriate handler.
    ///
    /// This is the single dispatch entry point — `event.rs` calls this
    /// and never matches on tool names itself.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        _sender: &str,
        session_id: Option<u64>,
    ) -> String {
        // Dispatch enforcement: reject tools not in the agent's whitelist.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.tools.is_empty()
            && !scope.tools.iter().any(|t| t.as_str() == name)
        {
            return format!("tool not available: {name}");
        }
        match name {
            "mcp" => self.dispatch_mcp(args, agent).await,
            "skill" => self.dispatch_skill(args, agent).await,
            "bash" => self.dispatch_bash(args, session_id).await,
            "delegate" => self.dispatch_delegate(args, agent).await,
            "recall" => self.dispatch_recall(args).await,
            "remember" => self.dispatch_remember(args).await,
            "memory" => self.dispatch_memory(args).await,
            "forget" => self.dispatch_forget(args).await,
            "ask_user" => self.dispatch_ask_user(args, session_id).await,
            name => format!("tool not available: {name}"),
        }
    }
}

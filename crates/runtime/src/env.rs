//! Env — the embeddable engine environment.
//!
//! [`Env`] composes skill, MCP, OS, and memory sub-hooks. It implements
//! `wcore::Hook` and provides the central `dispatch_tool` entry point. Server-
//! specific tools (`ask_user`, `delegate`) are routed through the
//! [`Host`](crate::host::Host).

use crate::{host::Host, mcp::McpHandler, memory::Memory, os, skill, skill::SkillHandler};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use wcore::{AgentConfig, AgentEvent, Hook, ToolRegistry, model::Message};

/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub(crate) tools: Vec<String>,
    pub(crate) members: Vec<String>,
    pub(crate) skills: Vec<String>,
    pub(crate) mcps: Vec<String>,
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

pub struct Env<H: Host = crate::NoHost> {
    pub(crate) skills: SkillHandler,
    pub(crate) mcp: McpHandler,
    pub(crate) cwd: PathBuf,
    pub(crate) memory: Option<Memory>,
    pub(crate) scopes: BTreeMap<String, AgentScope>,
    pub(crate) agent_descriptions: BTreeMap<String, String>,
    /// Host providing server-specific functionality.
    pub host: H,
}

impl<H: Host> Env<H> {
    /// Create a new Env with the given backends.
    pub fn new(
        skills: SkillHandler,
        mcp: McpHandler,
        cwd: PathBuf,
        memory: Option<Memory>,
        host: H,
    ) -> Self {
        Self {
            skills,
            mcp,
            cwd,
            memory,
            scopes: BTreeMap::new(),
            agent_descriptions: BTreeMap::new(),
            host,
        }
    }

    /// Access memory.
    pub fn memory(&self) -> Option<&Memory> {
        self.memory.as_ref()
    }

    /// List connected MCP servers with their tool names.
    pub fn mcp_servers(&self) -> Vec<(String, Vec<String>)> {
        self.mcp.cached_list()
    }

    /// Register an agent's scope for dispatch enforcement.
    pub fn register_scope(&mut self, name: String, config: &AgentConfig) {
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
    fn apply_scope(&self, config: &mut AgentConfig) {
        let has_scoping =
            !config.skills.is_empty() || !config.mcps.is_empty() || !config.members.is_empty();
        if !has_scoping {
            return;
        }

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

    /// Resolve a leading `/skill-name` command at the start of the message.
    fn resolve_slash_skill(&self, agent: &str, content: &str) -> String {
        let trimmed = content.trim_start();
        let Some(rest) = trimmed.strip_prefix('/') else {
            return content.to_owned();
        };

        let end = rest
            .find(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-')
            .unwrap_or(rest.len());
        let name = &rest[..end];
        let remainder = &rest[end..];

        if name.is_empty() || name.contains("..") {
            return content.to_owned();
        }

        // Enforce skill scope.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.skills.is_empty()
            && !scope.skills.iter().any(|s| s == name)
        {
            return content.to_owned();
        }

        // Try to load the skill from disk.
        for dir in &self.skills.skill_dirs {
            let skill_file = dir.join(name).join("SKILL.md");
            let Ok(file_content) = std::fs::read_to_string(&skill_file) else {
                continue;
            };
            let Ok(skill) = skill::loader::parse_skill_md(&file_content) else {
                continue;
            };
            let body = remainder.trim_start();
            let block = format!("<skill name=\"{name}\">\n{}\n</skill>", skill.body);
            return if body.is_empty() {
                block
            } else {
                format!("{body}\n\n{block}")
            };
        }

        content.to_owned()
    }

    /// Validate member scope and delegate to the bridge.
    async fn dispatch_delegate(&self, args: &str, agent: &str) -> String {
        let input: crate::task::Delegate = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.tasks.is_empty() {
            return "no tasks provided".to_owned();
        }
        // Enforce members scope for all target agents.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.members.is_empty()
        {
            for task in &input.tasks {
                if !scope.members.iter().any(|m| m == &task.agent) {
                    return format!("agent '{}' is not in your members list", task.agent);
                }
            }
        }
        self.host.dispatch_delegate(args, agent).await
    }

    /// Route a tool call by name to the appropriate handler.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        sender: &str,
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
            "bash" if sender.contains(':') => {
                "bash is only available in the command line interface".to_owned()
            }
            "bash" => self.dispatch_bash(args, session_id).await,
            "recall" => self.dispatch_recall(args).await,
            "remember" => self.dispatch_remember(args).await,
            "memory" => self.dispatch_memory(args).await,
            "forget" => self.dispatch_forget(args).await,
            "delegate" => self.dispatch_delegate(args, agent).await,
            "ask_user" => self.host.dispatch_ask_user(args, session_id).await,
            name => {
                self.host
                    .dispatch_custom_tool(name, args, agent, session_id)
                    .await
            }
        }
    }
}

impl<H: Host + 'static> Hook for Env<H> {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        config.system_prompt.push_str(&os::environment_block());

        if let Some(ref mem) = self.memory {
            let prompt = mem.build_prompt();
            if !prompt.is_empty() {
                config.system_prompt.push_str(&prompt);
            }
        }

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
            let visible: Vec<_> = if config.skills.is_empty() {
                reg.skills.iter().collect()
            } else {
                reg.skills
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
                    "Skills:\n\
                     When a <skill> tag appears in a message, it has been pre-loaded by the system. \
                     Follow its instructions directly — do not announce or re-load it.\n\
                     Use the skill tool to discover available skills or load one by name.\n{}",
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

        self.apply_scope(&mut config);
        config
    }

    fn preprocess(&self, agent: &str, content: &str) -> String {
        self.resolve_slash_skill(agent, content)
    }

    fn on_before_run(&self, agent: &str, session_id: u64, history: &[Message]) -> Vec<Message> {
        let mut messages = Vec::new();
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
        let cwd = self
            .host
            .session_cwd(session_id)
            .unwrap_or_else(|| self.cwd.clone());
        let mut cwd_msg = Message::user(format!(
            "<environment>\nworking_directory: {}\n</environment>",
            cwd.display()
        ));
        cwd_msg.auto_injected = true;
        messages.push(cwd_msg);
        if let Some(instructions) = discover_instructions(&cwd) {
            let mut msg = Message::user(format!("<instructions>\n{instructions}\n</instructions>"));
            msg.auto_injected = true;
            messages.push(msg);
        }
        messages
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.mcp.register_tools(tools);
        tools.insert_all(os::tool::tools());
        tools.insert_all(skill::tool::tools());
        tools.insert_all(crate::task::tools());
        tools.insert_all(crate::ask_user::tools());
        if self.memory.is_some() {
            tools.insert_all(crate::memory::tool::tools());
        }
    }

    fn on_event(&self, agent: &str, session_id: u64, event: &AgentEvent) {
        self.host.on_agent_event(agent, session_id, event);
    }
}

/// Collect layered `Crab.md` instructions: global (`~/.crabtalk/Crab.md`)
/// first, then any `Crab.md` files found walking up from `cwd` (root-first,
/// project-last so project instructions take precedence).
fn discover_instructions(cwd: &Path) -> Option<String> {
    let config_dir = &*wcore::paths::CONFIG_DIR;
    let mut layers = Vec::new();

    // Global instructions from config dir.
    let global = config_dir.join("Crab.md");
    if let Ok(content) = std::fs::read_to_string(&global) {
        layers.push(content);
    }

    // Walk up from CWD collecting project Crab.md files.
    let mut found = Vec::new();
    let mut dir = cwd;
    loop {
        let candidate = dir.join("Crab.md");
        if candidate.is_file()
            && !candidate.starts_with(config_dir)
            && let Ok(content) = std::fs::read_to_string(&candidate)
        {
            found.push(content);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    found.reverse();
    layers.extend(found);

    if layers.is_empty() {
        return None;
    }
    Some(layers.join("\n\n"))
}

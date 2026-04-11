//! Env — the embeddable engine environment.
//!
//! [`Env`] implements `wcore::Hook` and provides the central `dispatch_tool`
//! entry point. Tool handlers are registered dynamically at startup — there
//! is no hardcoded dispatch table.

use crate::{host::Host, memory::Memory, os};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use wcore::{
    AgentConfig, AgentEvent, Hook, ToolDispatch, ToolHandler, model::HistoryEntry, repos::Storage,
};

/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub tools: Vec<String>,
    pub members: Vec<String>,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
}

/// Base tools always included in every agent's whitelist.
const BASE_TOOLS: &[&str] = &["bash", "ask_user", "read", "edit"];

/// Skill discovery/loading tools.
const SKILL_TOOLS: &[&str] = &["skill"];

/// MCP discovery/call tools.
const MCP_TOOLS: &[&str] = &["mcp"];

/// Memory tools.
const MEMORY_TOOLS: &[&str] = &["recall", "remember", "memory", "forget"];

/// Task delegation tools.
const TASK_TOOLS: &[&str] = &["delegate"];

/// Per-conversation working directory overrides (shared with OS tool handlers).
pub type ConversationCwds = Arc<tokio::sync::Mutex<std::collections::HashMap<u64, PathBuf>>>;

pub struct Env<H: Host, S: Storage> {
    pub(crate) storage: Arc<S>,
    pub(crate) memory: Option<Arc<Memory<S>>>,
    pub(crate) cwd: PathBuf,
    pub scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
    pub(crate) agent_descriptions: RwLock<BTreeMap<String, String>>,
    /// Per-conversation CWD overrides.
    pub conversation_cwds: ConversationCwds,
    /// Host providing server-specific functionality.
    pub host: H,
    /// Dynamically registered tool handlers.
    handlers: BTreeMap<String, ToolHandler>,
}

impl<H: Host, S: Storage> Env<H, S> {
    /// Create a new Env with the given backends.
    pub fn new(
        storage: Arc<S>,
        cwd: PathBuf,
        memory: Option<Arc<Memory<S>>>,
        host: H,
        scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
        conversation_cwds: ConversationCwds,
    ) -> Self {
        Self {
            storage,
            memory,
            cwd,
            scopes,
            agent_descriptions: RwLock::new(BTreeMap::new()),
            conversation_cwds,
            host,
            handlers: BTreeMap::new(),
        }
    }

    /// Access memory.
    pub fn memory(&self) -> Option<&Memory<S>> {
        self.memory.as_deref()
    }

    /// Register a tool handler for dynamic dispatch.
    pub fn register_handler(&mut self, name: String, handler: ToolHandler) {
        self.handlers.insert(name, handler);
    }

    /// List connected MCP servers with their tool names.
    pub fn mcp_servers(&self) -> Vec<(String, Vec<String>)> {
        self.host.mcp_servers()
    }

    /// Register an agent's scope for dispatch enforcement.
    pub fn register_scope(&self, name: String, config: &AgentConfig) {
        if name != wcore::paths::DEFAULT_AGENT && !config.description.is_empty() {
            self.agent_descriptions
                .write()
                .expect("agent_descriptions lock poisoned")
                .insert(name.clone(), config.description.clone());
        }
        self.scopes.write().expect("scopes lock poisoned").insert(
            name,
            AgentScope {
                tools: config.tools.clone(),
                members: config.members.clone(),
                skills: config.skills.clone(),
                mcps: config.mcps.clone(),
            },
        );
    }

    /// Drop an agent's scope entry.
    pub fn unregister_scope(&self, name: &str) {
        self.scopes
            .write()
            .expect("scopes lock poisoned")
            .remove(name);
        self.agent_descriptions
            .write()
            .expect("agent_descriptions lock poisoned")
            .remove(name);
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
        {
            let scopes = self.scopes.read().expect("scopes lock poisoned");
            if let Some(scope) = scopes.get(agent)
                && !scope.skills.is_empty()
                && !scope.skills.iter().any(|s| s == name)
            {
                return content.to_owned();
            }
        }

        // Load via Storage.
        match self.storage.load_skill(name) {
            Ok(Some(skill)) => {
                let body = remainder.trim_start();
                let block = format!("<skill name=\"{name}\">\n{}\n</skill>", skill.body);
                if body.is_empty() {
                    block
                } else {
                    format!("{body}\n\n{block}")
                }
            }
            _ => content.to_owned(),
        }
    }

    /// Route a tool call by name to the appropriate handler.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        sender: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        // Dispatch enforcement: reject tools not in the agent's whitelist.
        {
            let scopes = self.scopes.read().expect("scopes lock poisoned");
            if let Some(scope) = scopes.get(agent)
                && !scope.tools.is_empty()
                && !scope.tools.iter().any(|t| t.as_str() == name)
            {
                return Err(format!("tool not available: {name}"));
            }
        }
        let handler = self
            .handlers
            .get(name)
            .ok_or_else(|| format!("tool not registered: {name}"))?
            .clone();
        handler(ToolDispatch {
            args: args.to_owned(),
            agent: agent.to_owned(),
            sender: sender.to_owned(),
            conversation_id,
        })
        .await
    }
}

impl<H: Host + 'static, S: Storage> Hook for Env<H, S> {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        config.system_prompt.push_str(&os::environment_block());

        if let Some(ref mem) = self.memory {
            let prompt = mem.build_prompt();
            if !prompt.is_empty() {
                config.system_prompt.push_str(&prompt);
            }
        }

        let mut hints = Vec::new();
        let mcp_servers = self.host.mcp_servers();
        if !mcp_servers.is_empty() {
            let names: Vec<&str> = mcp_servers.iter().map(|(n, _)| n.as_str()).collect();
            hints.push(format!(
                "MCP servers: {}. Use the mcp tool to list or call tools.",
                names.join(", ")
            ));
        }

        // List visible skills from storage.
        if let Ok(all_skills) = self.storage.list_skills() {
            let visible: Vec<&wcore::repos::Skill> = if config.skills.is_empty() {
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

    fn on_before_run(
        &self,
        agent: &str,
        conversation_id: u64,
        history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        let mut entries = Vec::new();
        let has_members = self
            .scopes
            .read()
            .expect("scopes lock poisoned")
            .get(agent)
            .is_some_and(|s| !s.members.is_empty());
        if has_members {
            let descriptions = self
                .agent_descriptions
                .read()
                .expect("agent_descriptions lock poisoned");
            if !descriptions.is_empty() {
                let mut block = String::from("<agents>\n");
                for (name, desc) in descriptions.iter() {
                    block.push_str(&format!("- {name}: {desc}\n"));
                }
                block.push_str("</agents>");
                entries.push(HistoryEntry::user(block).auto_injected());
            }
        }
        if let Some(ref mem) = self.memory {
            entries.extend(mem.before_run(history));
        }
        let cwd = self
            .conversation_cwds
            .try_lock()
            .ok()
            .and_then(|m| m.get(&conversation_id).cloned())
            .unwrap_or_else(|| self.cwd.clone());
        entries.push(
            HistoryEntry::user(format!(
                "<environment>\nworking_directory: {}\n</environment>",
                cwd.display()
            ))
            .auto_injected(),
        );
        if let Some(instructions) = self.host.discover_instructions(&cwd) {
            entries.push(
                HistoryEntry::user(format!("<instructions>\n{instructions}\n</instructions>"))
                    .auto_injected(),
            );
        }
        if history.iter().any(|e| !e.agent.is_empty()) {
            entries.push(
                HistoryEntry::user(
                    "Messages wrapped in <from agent=\"...\"> tags are from guest agents \
                     who were consulted in this conversation. Continue responding as yourself."
                        .to_string(),
                )
                .auto_injected(),
            );
        }
        entries
    }

    fn on_event(&self, agent: &str, conversation_id: u64, event: &AgentEvent) {
        self.host.on_agent_event(agent, conversation_id, event);
    }
}

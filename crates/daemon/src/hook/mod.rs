//! Stateful Hook implementation for the daemon.
//!
//! [`DaemonHook`] composes memory, skill, MCP, and OS sub-hooks.
//! `on_build_agent` delegates to skills and memory; `on_register_tools`
//! delegates to all sub-hooks in sequence. `dispatch_tool` routes every
//! agent tool call by name — the single entry point from `event.rs`.

use crate::hook::{
    os::OsHook,
    skill::{SkillHandler, loader},
};
use mcp::McpHandler;
use memory::InMemory;
use schemars::JsonSchema;
use serde::Deserialize;
use wcore::{
    AgentConfig, AgentEvent, Hook, Memory, RecallInput, RecallOptions, RememberInput, ToolRegistry,
    model::Tool,
};

pub mod mcp;
pub mod os;
pub mod skill;

/// Stateful Hook implementation for the daemon.
///
/// Composes memory, skill, MCP, and OS sub-hooks. Each sub-hook
/// self-registers its tools via `on_register_tools`. All tool dispatch
/// is routed through `dispatch_tool`.
pub struct DaemonHook {
    pub memory: InMemory,
    pub skills: SkillHandler,
    pub mcp: McpHandler,
    pub os: OsHook,
}

impl DaemonHook {
    /// Create a new DaemonHook with the given backends.
    pub fn new(memory: InMemory, skills: SkillHandler, mcp: McpHandler) -> Self {
        Self {
            memory,
            skills,
            mcp,
            os: OsHook,
        }
    }

    /// Route a tool call by name to the appropriate handler.
    ///
    /// This is the single dispatch entry point — `event.rs` calls this
    /// and never matches on tool names itself. Unrecognised names are
    /// forwarded to the MCP bridge after a warn-level log.
    pub async fn dispatch_tool(&self, name: &str, args: &str) -> String {
        match name {
            "remember" => self.dispatch_remember(args).await,
            "recall" => self.dispatch_recall(args).await,
            "search_mcp" => self.dispatch_search_mcp(args).await,
            "call_mcp_tool" => self.dispatch_call_mcp_tool(args).await,
            "search_skill" => self.dispatch_search_skill(args).await,
            "load_skill" => self.dispatch_load_skill(args).await,
            "read" => self.os.dispatch_read(args).await,
            "write" => self.os.dispatch_write(args).await,
            name => {
                tracing::debug!(tool = name, "forwarding tool to MCP bridge");
                let bridge = self.mcp.bridge().await;
                bridge.call(name, args).await
            }
        }
    }

    // ── Memory tools ─────────────────────────────────────────────────

    async fn dispatch_remember(&self, args: &str) -> String {
        let input: RememberInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.key.is_empty() {
            return "missing required field: key".to_owned();
        }
        let key = input.key.clone();
        match self.memory.store(input.key, input.value).await {
            Ok(()) => format!("remembered: {key}"),
            Err(e) => format!("failed to store: {e}"),
        }
    }

    async fn dispatch_recall(&self, args: &str) -> String {
        let input: RecallInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let limit = input.limit.unwrap_or(10) as usize;
        let options = RecallOptions {
            limit,
            ..Default::default()
        };
        match self.memory.recall(&input.query, options).await {
            Ok(entries) if entries.is_empty() => "no memories found".to_owned(),
            Ok(entries) => entries
                .iter()
                .map(|e| format!("{}: {}", e.key, e.value))
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => format!("recall failed: {e}"),
        }
    }

    // ── MCP tools ────────────────────────────────────────────────────

    async fn dispatch_search_mcp(&self, args: &str) -> String {
        let input: SearchMcpInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let query = input.query.to_lowercase();
        let bridge = self.mcp.bridge().await;
        let tools = bridge.tools().await;
        let matches: Vec<String> = tools
            .iter()
            .filter(|t| {
                t.name.to_lowercase().contains(&query)
                    || t.description.to_lowercase().contains(&query)
            })
            .map(|t| format!("{}: {}", t.name, t.description))
            .collect();
        if matches.is_empty() {
            "no tools found".to_owned()
        } else {
            matches.join("\n")
        }
    }

    async fn dispatch_call_mcp_tool(&self, args: &str) -> String {
        let input: CallMcpToolInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let tool_args = input.args.unwrap_or_default();
        let bridge = self.mcp.bridge().await;
        bridge.call(&input.name, &tool_args).await
    }

    // ── Skill tools ──────────────────────────────────────────────────

    async fn dispatch_search_skill(&self, args: &str) -> String {
        let input: SearchSkillInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let query = input.query.to_lowercase();
        let registry = self.skills.registry.lock().unwrap();
        let matches: Vec<String> = registry
            .skills()
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query)
                    || s.description.to_lowercase().contains(&query)
            })
            .map(|s| format!("{}: {}", s.name, s.description))
            .collect();
        if matches.is_empty() {
            "no skills found".to_owned()
        } else {
            matches.join("\n")
        }
    }

    async fn dispatch_load_skill(&self, args: &str) -> String {
        let input: LoadSkillInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let name = &input.name;
        // Guard against path traversal in the skill name.
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return format!("invalid skill name: {name}");
        }
        let skill_dir = self.skills.skills_dir.join(name);
        let skill_file = skill_dir.join("SKILL.md");
        let content = match tokio::fs::read_to_string(&skill_file).await {
            Ok(c) => c,
            Err(_) => return format!("skill not found: {name}"),
        };
        let skill = match loader::parse_skill_md(&content) {
            Ok(s) => s,
            Err(e) => return format!("failed to parse skill: {e}"),
        };
        let body = skill.body.clone();
        self.skills.registry.lock().unwrap().add(skill);
        let dir_path = skill_dir.display();
        format!("{body}\n\nSkill directory: {dir_path}")
    }
}

impl Hook for DaemonHook {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        self.memory.on_build_agent(config)
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        self.memory.on_register_tools(tools).await;
        self.mcp.on_register_tools(tools).await;
        self.os.on_register_tools(tools).await;
        self.register_system_tools(tools);
    }

    fn on_event(&self, agent: &str, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta(text) => {
                tracing::trace!(%agent, text_len = text.len(), "agent text delta");
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
            }
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct SearchMcpInput {
    /// Keyword to match tool names and descriptions
    query: String,
}

#[derive(Deserialize, JsonSchema)]
struct CallMcpToolInput {
    /// Tool name
    name: String,
    /// JSON-encoded arguments string
    args: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchSkillInput {
    /// Keyword to match skill names and descriptions
    query: String,
}

#[derive(Deserialize, JsonSchema)]
struct LoadSkillInput {
    /// Skill name
    name: String,
}

impl DaemonHook {
    /// Register MCP and skill discovery tool schemas.
    fn register_system_tools(&self, tools: &mut ToolRegistry) {
        tools.insert(Tool {
            name: "search_mcp".into(),
            description: "Search available MCP tools by keyword.".into(),
            parameters: schemars::schema_for!(SearchMcpInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "call_mcp_tool".into(),
            description: "Call an MCP tool by name with JSON-encoded arguments.".into(),
            parameters: schemars::schema_for!(CallMcpToolInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "search_skill".into(),
            description: "Search available skills by keyword. Returns name and description only."
                .into(),
            parameters: schemars::schema_for!(SearchSkillInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "load_skill".into(),
            description: "Load a skill by name. Returns its instructions and the skill directory path for resolving relative file references.".into(),
            parameters: schemars::schema_for!(LoadSkillInput),
            strict: false,
        });
    }
}

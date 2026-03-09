//! Stateful Hook implementation for the daemon.
//!
//! [`DaemonHook`] composes memory, skill, MCP, and OS sub-hooks.
//! `on_build_agent` delegates to skills and memory; `on_register_tools`
//! delegates to all sub-hooks in sequence. `dispatch_tool` routes every
//! agent tool call by name — the single entry point from `event.rs`.

use crate::{
    config::PermissionConfig,
    hook::{
        mcp::{CallMcpToolInput, McpHandler, SearchMcpInput},
        memory::MemoryHook,
        os::OsHook,
        skill::{LoadSkillInput, SearchSkillInput, SkillHandler, loader},
        task::TaskRegistry,
    },
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use wcore::{AgentConfig, AgentEvent, Hook, ToolRegistry, model::Tool};

pub mod mcp;
pub mod memory;
pub mod os;
pub mod skill;
pub mod task;

/// Stateful Hook implementation for the daemon.
///
/// Composes memory, skill, MCP, and OS sub-hooks. Each sub-hook
/// self-registers its tools via `on_register_tools`. All tool dispatch
/// is routed through `dispatch_tool`.
pub struct DaemonHook {
    pub memory: MemoryHook,
    pub skills: SkillHandler,
    pub mcp: McpHandler,
    pub os: OsHook,
    pub tasks: Arc<Mutex<TaskRegistry>>,
    pub permissions: PermissionConfig,
    /// Whether the daemon is running as the `walrus` OS user (sandbox active).
    pub sandboxed: bool,
}

/// OS tool names — bypass permission check when running in sandbox mode.
const OS_TOOLS: &[&str] = &["read", "write", "bash"];

impl DaemonHook {
    /// Create a new DaemonHook with the given backends.
    pub fn new(
        memory: MemoryHook,
        skills: SkillHandler,
        mcp: McpHandler,
        tasks: Arc<Mutex<TaskRegistry>>,
        permissions: PermissionConfig,
        sandboxed: bool,
    ) -> Self {
        Self {
            memory,
            skills,
            mcp,
            os: OsHook::new(),
            tasks,
            permissions,
            sandboxed,
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
    ) -> String {
        // Permission check — skip for OS tools when running in sandbox mode.
        let skip_perm = self.sandboxed && OS_TOOLS.contains(&name);
        if !skip_perm {
            use crate::config::ToolPermission;
            match self.permissions.resolve(agent, name) {
                ToolPermission::Deny => {
                    return format!("permission denied: {name}");
                }
                ToolPermission::Ask => {
                    if let Some(tid) = task_id {
                        // Truncate args for the question to avoid huge messages.
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
                                    return format!("permission denied: {name}");
                                }
                                Err(_) => {
                                    return format!("permission denied: {name} (inbox dropped)");
                                }
                                _ => {} // approved or any other response → proceed
                            }
                        }
                    }
                    // No task_id → can't block, treat as Allow.
                }
                ToolPermission::Allow => {}
            }
        }
        match name {
            "remember" => self.memory.dispatch_remember(args, agent).await,
            "recall" => self.memory.dispatch_recall(args, agent).await,
            "relate" => self.memory.dispatch_relate(args, agent).await,
            "connections" => self.memory.dispatch_connections(args, agent).await,
            "compact" => self.memory.dispatch_compact(agent).await,
            "__journal__" => self.memory.dispatch_journal(args, agent).await,
            "distill" => self.memory.dispatch_distill(args, agent).await,
            "search_mcp" => self.dispatch_search_mcp(args).await,
            "call_mcp_tool" => self.dispatch_call_mcp_tool(args).await,
            "search_skill" => self.dispatch_search_skill(args).await,
            "load_skill" => self.dispatch_load_skill(args).await,
            "read" => self.os.dispatch_read(args).await,
            "write" => self.os.dispatch_write(args).await,
            "bash" => self.os.dispatch_bash(args).await,
            "spawn_task" => self.dispatch_spawn_task(args, agent, task_id).await,
            "check_tasks" => self.dispatch_check_tasks(args).await,
            "create_task" => self.dispatch_create_task(args, agent).await,
            "ask_user" => self.dispatch_ask_user(args, task_id).await,
            "await_tasks" => self.dispatch_await_tasks(args, task_id).await,
            name => {
                tracing::debug!(tool = name, "forwarding tool to MCP bridge");
                let bridge = self.mcp.bridge().await;
                bridge.call(name, args).await
            }
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
        let registry = self.skills.registry.lock().await;
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
        self.skills.registry.lock().await.add(skill);
        let dir_path = skill_dir.display();
        format!("{body}\n\nSkill directory: {dir_path}")
    }

    // ── Task tools ─────────────────────────────────────────────────

    async fn dispatch_spawn_task(
        &self,
        args: &str,
        agent: &str,
        parent_task_id: Option<u64>,
    ) -> String {
        let input: SpawnTaskInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let registry = self.tasks.clone();
        let (task_id, status) = registry.lock().await.submit(
            input.agent.into(),
            input.message,
            agent.into(),
            parent_task_id,
            registry.clone(),
        );
        serde_json::json!({ "task_id": task_id, "status": status.to_string() }).to_string()
    }

    async fn dispatch_check_tasks(&self, args: &str) -> String {
        let input: CheckTasksInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let status_filter = input.status.as_deref().and_then(parse_task_status);
        let registry = self.tasks.lock().await;
        let tasks = registry.list(
            input.agent.as_deref(),
            status_filter,
            input.parent_id.map(Some),
        );
        let entries: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "task_id": t.id,
                    "agent": t.agent.as_str(),
                    "status": t.status.to_string(),
                    "description": t.description,
                    "parent_id": t.parent_id,
                    "result": t.result,
                    "error": t.error,
                    "created_by": t.created_by.as_str(),
                    "alive_secs": t.created_at.elapsed().as_secs(),
                    "prompt_tokens": t.prompt_tokens,
                    "completion_tokens": t.completion_tokens,
                })
            })
            .collect();
        serde_json::to_string(&entries).unwrap_or_else(|e| format!("serialization error: {e}"))
    }

    async fn dispatch_create_task(&self, args: &str, agent: &str) -> String {
        let input: CreateTaskInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let mut registry = self.tasks.lock().await;
        let task_id = registry.create(
            input.agent.into(),
            input.description,
            agent.into(),
            None,
            task::TaskStatus::Queued,
            false,
        );
        serde_json::json!({ "task_id": task_id, "status": "queued" }).to_string()
    }

    async fn dispatch_ask_user(&self, args: &str, task_id: Option<u64>) -> String {
        let input: AskUserInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let Some(tid) = task_id else {
            return "ask_user can only be called from within a task context".to_owned();
        };
        let rx = {
            let mut registry = self.tasks.lock().await;
            match registry.block(tid, input.question) {
                Some(rx) => rx,
                None => return format!("task {tid} not found"),
            }
        };
        match rx.await {
            Ok(response) => response,
            Err(_) => "user did not respond (channel closed)".to_owned(),
        }
    }

    async fn dispatch_await_tasks(&self, args: &str, task_id: Option<u64>) -> String {
        let input: AwaitTasksInput = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.task_ids.is_empty() {
            return "no task IDs provided".to_owned();
        }
        // Subscribe to status changes for each requested task.
        let mut receivers = Vec::new();
        {
            let registry = self.tasks.lock().await;
            for &tid in &input.task_ids {
                match registry.subscribe_status(tid) {
                    Some(rx) => receivers.push((tid, rx)),
                    None => return format!("task {tid} not found"),
                }
            }
        }
        // If running in a task context, mark ourselves as blocked.
        if let Some(tid) = task_id {
            let mut registry = self.tasks.lock().await;
            registry.set_status(tid, task::TaskStatus::Blocked);
        }
        // Wait for all tasks to reach Finished or Failed.
        for (_, rx) in &mut receivers {
            let mut rx = rx.clone();
            loop {
                let status = *rx.borrow_and_update();
                if status == task::TaskStatus::Finished || status == task::TaskStatus::Failed {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
        // Unblock ourselves.
        if let Some(tid) = task_id {
            let mut registry = self.tasks.lock().await;
            registry.set_status(tid, task::TaskStatus::InProgress);
        }
        // Collect results.
        let registry = self.tasks.lock().await;
        let results: Vec<serde_json::Value> = input
            .task_ids
            .iter()
            .map(|&tid| {
                if let Some(t) = registry.get(tid) {
                    serde_json::json!({
                        "task_id": tid,
                        "status": t.status.to_string(),
                        "result": t.result,
                        "error": t.error,
                    })
                } else {
                    serde_json::json!({ "task_id": tid, "status": "not_found" })
                }
            })
            .collect();
        serde_json::to_string(&results).unwrap_or_else(|e| format!("serialization error: {e}"))
    }
}

/// Input for the `spawn_task` tool.
#[derive(Deserialize, schemars::JsonSchema)]
struct SpawnTaskInput {
    /// Target agent name to delegate the task to.
    agent: String,
    /// Message/instruction for the target agent.
    message: String,
}

/// Input for the `check_tasks` tool.
#[derive(Deserialize, schemars::JsonSchema)]
struct CheckTasksInput {
    /// Filter by agent name.
    #[serde(default)]
    agent: Option<String>,
    /// Filter by status (queued, in_progress, blocked, finished, failed).
    #[serde(default)]
    status: Option<String>,
    /// Filter by parent task ID.
    #[serde(default)]
    parent_id: Option<u64>,
}

/// Input for the `create_task` tool.
#[derive(Deserialize, schemars::JsonSchema)]
struct CreateTaskInput {
    /// Target agent name.
    agent: String,
    /// Human-readable task description.
    description: String,
}

/// Input for the `ask_user` tool.
#[derive(Deserialize, schemars::JsonSchema)]
struct AskUserInput {
    /// Question to ask the user.
    question: String,
}

/// Input for the `await_tasks` tool.
#[derive(Deserialize, schemars::JsonSchema)]
struct AwaitTasksInput {
    /// Task IDs to wait for.
    task_ids: Vec<u64>,
}

/// Parse a status string into a `TaskStatus`.
fn parse_task_status(s: &str) -> Option<task::TaskStatus> {
    match s {
        "queued" => Some(task::TaskStatus::Queued),
        "in_progress" => Some(task::TaskStatus::InProgress),
        "blocked" => Some(task::TaskStatus::Blocked),
        "finished" => Some(task::TaskStatus::Finished),
        "failed" => Some(task::TaskStatus::Failed),
        _ => None,
    }
}

impl Hook for DaemonHook {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        self.memory.on_build_agent(config)
    }

    fn on_compact(&self, prompt: &mut String) {
        self.memory.on_compact(prompt);
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
        tools.insert(Tool {
            name: "spawn_task".into(),
            description: "Delegate an async task to another agent. Returns task_id and status (in_progress or queued). Use check_tasks to monitor progress.".into(),
            parameters: schemars::schema_for!(SpawnTaskInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "check_tasks".into(),
            description: "Query the task registry. Filterable by agent, status, parent_id. Returns up to 16 most recent tasks.".into(),
            parameters: schemars::schema_for!(CheckTasksInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "create_task".into(),
            description:
                "Queue a task for later pickup (heartbeat or manual). Always starts as queued."
                    .into(),
            parameters: schemars::schema_for!(CreateTaskInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "ask_user".into(),
            description: "Ask the user a question. Blocks the current task until the user responds. Only works within a task context.".into(),
            parameters: schemars::schema_for!(AskUserInput),
            strict: false,
        });
        tools.insert(Tool {
            name: "await_tasks".into(),
            description:
                "Block until the specified tasks finish. Returns collected results for each task."
                    .into(),
            parameters: schemars::schema_for!(AwaitTasksInput),
            strict: false,
        });
    }
}

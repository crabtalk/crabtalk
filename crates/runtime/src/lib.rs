//! Walrus runtime: the top-level orchestrator.
//!
//! The [`Runtime`] is the entry point for the agent framework. It holds
//! the LLM provider (configured via [`Hook`]), agent configurations,
//! tool handlers, and manages sessions internally.
//!
//! # Example
//!
//! ```rust,ignore
//! use walrus_runtime::prelude::*;
//!
//! let provider = MyProvider::new(client, &key)?;
//! let mut runtime = Runtime::<MyHook>::new(General::default(), provider, InMemory::new());
//! runtime.add_agent(Agent::new("assistant").system_prompt("You are helpful."));
//! let response = runtime.send_to("assistant", Message::user("hello")).await?;
//! ```

pub use agent::{Agent, InMemory, Memory, NoEmbedder, Skill, SkillTier};
pub use hook::{DEFAULT_COMPACT_PROMPT, DEFAULT_FLUSH_PROMPT, Hook};
pub use llm::{Client, General, Message, Response, Role, StreamChunk, Tool};
pub use mcp::McpBridge;
pub use skills::{SkillRegistry, parse_skill_md};
pub use team::{build_team, extract_input, worker_tool};

use anyhow::Result;
use compact_str::CompactString;
use futures_core::Stream;
use futures_util::StreamExt;
use llm::{Config, FinishReason, LLM, ToolCall, ToolChoice, estimate_tokens};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

pub mod hook;
pub mod mcp;
pub mod skills;
pub mod team;

/// Re-exports of the most commonly used types.
pub mod prelude {
    pub use crate::{
        Agent, General, Hook, InMemory, Message, Response, Role, Runtime, SkillRegistry,
        StreamChunk, Tool,
    };
}

pub const MAX_TOOL_CALLS: usize = 16;

/// A type-erased async tool handler.
pub type Handler =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync>;

/// Private session state, keyed by agent name in the sessions map.
struct Session {
    messages: Vec<Message>,
    compaction_count: usize,
}

impl Session {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            compaction_count: 0,
        }
    }
}

/// The walrus runtime — top-level orchestrator.
///
/// Generic over `H: Hook` for type-level configuration of the LLM provider,
/// memory backend, and compaction prompts.
pub struct Runtime<H: Hook> {
    provider: H::Provider,
    config: General,
    memory: Arc<H::Memory>,
    skills: Option<Arc<SkillRegistry>>,
    mcp: Option<Arc<McpBridge>>,
    tools: BTreeMap<CompactString, (Tool, Handler)>,
    agents: BTreeMap<CompactString, Agent>,
    sessions: BTreeMap<CompactString, Session>,
}

impl<H: Hook + 'static> Runtime<H> {
    /// Create a new runtime with the given config, provider, and memory.
    pub fn new(config: General, provider: H::Provider, memory: H::Memory) -> Self {
        let memory = Arc::new(memory);
        let mut rt = Self {
            provider,
            config,
            memory: Arc::clone(&memory),
            skills: None,
            mcp: None,
            tools: BTreeMap::new(),
            agents: BTreeMap::new(),
            sessions: BTreeMap::new(),
        };

        // Auto-register the "remember" tool (DD#23).
        let mem = memory;
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Memory key" },
                "value": { "type": "string", "description": "Value to remember" }
            },
            "required": ["key", "value"]
        });
        let tool = Tool {
            name: "remember".into(),
            description: "Store a key-value pair in memory.".into(),
            parameters: serde_json::from_value(schema).unwrap(),
            strict: false,
        };
        rt.register(tool, move |args| {
            let mem = Arc::clone(&mem);
            async move {
                let parsed: serde_json::Value = match serde_json::from_str(&args) {
                    Ok(v) => v,
                    Err(e) => return format!("invalid arguments: {e}"),
                };
                let key = parsed["key"].as_str().unwrap_or("");
                let value = parsed["value"].as_str().unwrap_or("");
                match mem.store(key.to_owned(), value.to_owned()).await {
                    Ok(()) => format!("remembered: {key}"),
                    Err(e) => format!("failed to store: {e}"),
                }
            }
        });

        rt
    }

    /// Set the skill registry for this runtime (builder-style).
    pub fn with_skills(mut self, registry: SkillRegistry) -> Self {
        self.skills = Some(Arc::new(registry));
        self
    }

    /// Set the skill registry for this runtime (mutable setter).
    pub fn set_skills(&mut self, registry: SkillRegistry) {
        self.skills = Some(Arc::new(registry));
    }

    /// Connect an MCP bridge to this runtime.
    pub fn connect_mcp(&mut self, bridge: McpBridge) {
        self.mcp = Some(Arc::new(bridge));
    }

    /// Get a reference to the MCP bridge, if connected.
    pub fn mcp_bridge(&self) -> Option<&McpBridge> {
        self.mcp.as_deref()
    }

    /// Register all MCP tools from the connected bridge into the tool registry.
    ///
    /// Each MCP tool becomes a regular tool, dispatched via [`McpBridge::call`].
    pub async fn register_mcp_tools(&mut self) -> anyhow::Result<()> {
        let bridge = self
            .mcp
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no MCP bridge connected"))?;
        let mcp_tools = bridge.tools().await;
        for tool in mcp_tools {
            let name = tool.name.clone();
            let bridge = Arc::clone(bridge);
            self.tools.insert(
                name.clone(),
                (
                    tool,
                    Arc::new(move |args: String| {
                        let bridge = Arc::clone(&bridge);
                        let name = name.clone();
                        Box::pin(async move { bridge.call(&name, &args).await })
                    }),
                ),
            );
        }
        Ok(())
    }

    /// Register an agent.
    pub fn add_agent(&mut self, agent: Agent) {
        self.agents.insert(agent.name.clone(), agent);
    }

    /// Get a registered agent by name.
    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.get(name)
    }

    /// Register a tool with its handler.
    pub fn register<F, Fut>(&mut self, tool: Tool, handler: F)
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = String> + Send + 'static,
    {
        let name = tool.name.clone();
        let handler: Handler = Arc::new(move |args| Box::pin(handler(args)));
        self.tools.insert(name, (tool, handler));
    }

    /// Context window limit for the current provider/model.
    pub fn context_limit(&self) -> usize {
        H::context_limit(&self.config)
    }

    /// Clear the session for a named agent, resetting conversation history.
    pub fn clear_session(&mut self, agent: &str) {
        self.sessions.remove(agent);
    }

    /// Resolve tool schemas and handlers for the given tool names.
    ///
    /// Supports glob prefixes (DD#21): a name ending in `*` expands against
    /// all registered tool names by prefix match. No-match globs are logged.
    pub fn resolve_tools(&self, names: &[CompactString]) -> Vec<(Tool, Handler)> {
        let mut resolved = Vec::new();
        for name in names {
            if let Some(prefix) = name.strip_suffix('*') {
                let mut matched = false;
                for (tool_name, (tool, handler)) in &self.tools {
                    if tool_name.starts_with(prefix) {
                        resolved.push((tool.clone(), Arc::clone(handler)));
                        matched = true;
                    }
                }
                if !matched {
                    tracing::warn!("glob pattern '{name}' matched no registered tools");
                }
            } else if let Some((tool, handler)) = self.tools.get(name.as_str()) {
                resolved.push((tool.clone(), Arc::clone(handler)));
            }
        }
        resolved
    }

    /// Resolve tool schemas for the given tool names (schemas only).
    ///
    /// Thin wrapper over [`resolve_tools`] that discards handlers.
    pub fn resolve(&self, names: &[CompactString]) -> Vec<Tool> {
        self.resolve_tools(names)
            .into_iter()
            .map(|(tool, _)| tool)
            .collect()
    }

    /// Dispatch tool calls and collect results as tool messages.
    pub async fn dispatch(&self, calls: &[ToolCall]) -> Vec<Message> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            let output = if let Some((_, handler)) = self.tools.get(call.function.name.as_str()) {
                handler(call.function.arguments.clone()).await
            } else {
                format!("function {} not available", call.function.name)
            };
            results.push(Message::tool(output, call.id.clone()));
        }
        results
    }

    /// Send a message to a named agent, creating or reusing a session.
    pub async fn send_to(&mut self, agent: &str, message: Message) -> Result<Response> {
        if !self.agents.contains_key(agent) {
            anyhow::bail!("agent '{agent}' not registered");
        }
        let key = CompactString::from(agent);
        let mut session = self.sessions.remove(&key).unwrap_or_else(Session::new);
        let result = self.send_inner(agent, &mut session, message).await;
        self.sessions.insert(key, session);
        result
    }

    /// Stream a message to a named agent, creating or reusing a session.
    pub fn stream_to<'a>(
        &'a mut self,
        agent: &'a str,
        message: Message,
    ) -> impl Stream<Item = Result<StreamChunk>> + 'a {
        async_stream::try_stream! {
            if !self.agents.contains_key(agent) {
                Err(anyhow::anyhow!("agent '{agent}' not registered"))?;
            }
            let key = CompactString::from(agent);
            let old_session = self.sessions.remove(&key);
            let compaction_count = old_session.as_ref().map_or(0, |s| s.compaction_count);
            let mut messages = old_session.map(|s| s.messages).unwrap_or_default();

            let agent_config = self.agents.get(agent).cloned().unwrap();
            let tools = self.resolve(&agent_config.tools);
            messages.push(message);
            let mut tool_choice = ToolChoice::Auto;

            'outer: for _ in 0..MAX_TOOL_CALLS {
                let api_msgs = self.api_messages_from(agent, &messages).await;
                let cfg = self.build_config(&tools, tool_choice.clone());
                let mut builder = Message::builder(Role::Assistant);

                let inner = self.provider.stream(self.provider_cfg(cfg), &api_msgs, self.config.usage);
                futures_util::pin_mut!(inner);

                while let Some(result) = inner.next().await {
                    let chunk = result?;
                    let reason = chunk.reason().cloned();

                    if builder.accept(&chunk) {
                        yield chunk;
                    }

                    if let Some(reason) = reason {
                        match reason {
                            FinishReason::Stop => break 'outer,
                            FinishReason::ToolCalls => break,
                            reason => Err(anyhow::anyhow!("unexpected finish reason: {reason:?}"))?,
                        }
                    }
                }

                let message = builder.build();
                if message.tool_calls.is_empty() {
                    messages.push(message);
                    break;
                }

                let result = self.dispatch(&message.tool_calls).await;
                messages.push(message);
                messages.extend(result);
                tool_choice = ToolChoice::None;

                // Emit a newline so consumers see a break between
                // pre-tool-call text and the next response.
                yield StreamChunk::separator();
            }

            self.sessions.insert(key, Session { messages, compaction_count });
        }
    }

    /// Get a reference to the memory backend.
    pub fn memory(&self) -> &H::Memory {
        &self.memory
    }

    /// Get a clone of the memory Arc (for team delegation).
    pub fn memory_arc(&self) -> Arc<H::Memory> {
        Arc::clone(&self.memory)
    }

    /// Get a reference to the provider.
    pub fn provider(&self) -> &H::Provider {
        &self.provider
    }

    /// Get a reference to the general config.
    pub fn config(&self) -> &General {
        &self.config
    }

    /// Get a shared reference to the skill registry, if one is set.
    pub fn skills(&self) -> Option<&Arc<SkillRegistry>> {
        self.skills.as_ref()
    }

    // --- Private helpers ---

    /// Estimate current token usage for a session.
    fn estimate_session_tokens(&self, agent: &str, session: &Session) -> usize {
        let system_tokens = self
            .agents
            .get(agent)
            .map(|a| (a.system_prompt.len() / 4).max(1))
            .unwrap_or(0);
        system_tokens + estimate_tokens(&session.messages)
    }

    /// Check if a session is approaching the context limit.
    fn needs_compaction(&self, agent: &str, session: &Session) -> bool {
        let usage = self.estimate_session_tokens(agent, session);
        let limit = self.context_limit();
        usage > (limit * 4 / 5)
    }

    /// Build the message list for an API request.
    ///
    /// Injects the agent system prompt (enriched with memory) if not already present.
    async fn api_messages(&self, agent: &str, session: &Session) -> Vec<Message> {
        self.api_messages_from(agent, &session.messages).await
    }

    /// Build the message list for an API request from raw messages.
    ///
    /// Constructs the system prompt (base + memory + skills) and prepends it,
    /// then clones source messages in a single pass stripping reasoning_content.
    pub async fn api_messages_from(&self, agent: &str, source: &[Message]) -> Vec<Message> {
        let agent_config = match self.agents.get(agent) {
            Some(a) => a,
            None => return source.to_vec(),
        };

        // Build system prompt: base + memory context + skill bodies.
        let mut system_prompt = agent_config.system_prompt.clone();
        let last_user_msg = source
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let memory_context = self.memory.compile_relevant(last_user_msg).await;
        if !memory_context.is_empty() {
            system_prompt = format!("{system_prompt}\n\n{memory_context}");
        }

        if let Some(registry) = &self.skills {
            for skill in registry.find_by_tags(&agent_config.skill_tags) {
                if !skill.body.is_empty() {
                    system_prompt.push_str("\n\n");
                    system_prompt.push_str(&skill.body);
                }
            }
        }

        // Single-pass construction: system prompt + source messages.
        let needs_system = source.first().map(|m| m.role) != Some(Role::System);
        let extra = if needs_system { 1 } else { 0 };
        let mut messages = Vec::with_capacity(source.len() + extra);

        if needs_system {
            messages.push(Message::system(&system_prompt));
        }
        for m in source {
            let mut cloned = m.clone();
            if cloned.tool_calls.is_empty() {
                cloned.reasoning_content = String::new();
            }
            messages.push(cloned);
        }

        messages
    }

    /// Automatic compaction: flush memory then summarize conversation.
    ///
    /// Called at the start of `send_to()` and each loop iteration in `stream_to()`.
    /// Uses `H::flush()` and `H::compact()` as static calls (no Hook instance).
    async fn maybe_compact(&self, agent: &str, session: &mut Session) {
        if !self.needs_compaction(agent, session) {
            return;
        }

        // 1. Memory flush: extract durable facts via "remember" tool.
        let flush_prompt = H::flush();
        if !flush_prompt.is_empty() {
            let remember_tool = self.resolve(&["remember".into()]);
            let mut flush_messages = session.messages.clone();
            flush_messages.insert(0, Message::system(flush_prompt));
            let cfg = self
                .config
                .clone()
                .with_tools(remember_tool)
                .with_tool_choice(ToolChoice::Auto);
            let pcfg = self.provider_cfg(cfg);
            match self.provider.send(&pcfg, &flush_messages).await {
                Ok(response) => {
                    if let Some(msg) = response.message()
                        && !msg.tool_calls.is_empty()
                    {
                        self.dispatch(&msg.tool_calls).await;
                    }
                }
                Err(e) => tracing::warn!("memory flush failed during compaction: {e}"),
            }
        }

        // 2. Summarize conversation history.
        let compact_prompt = H::compact();
        if !compact_prompt.is_empty() {
            let mut summary_messages = session.messages.clone();
            summary_messages.insert(0, Message::system(compact_prompt));
            let cfg = self
                .config
                .clone()
                .with_tools(vec![])
                .with_tool_choice(ToolChoice::None);
            let pcfg = self.provider_cfg(cfg);
            match self.provider.send(&pcfg, &summary_messages).await {
                Ok(response) => {
                    let summary = response.content().cloned().unwrap_or_default();
                    session.messages.clear();
                    session
                        .messages
                        .push(Message::assistant(&summary, None, None));
                    session.compaction_count += 1;
                }
                Err(e) => tracing::warn!("compaction summarization failed: {e}"),
            }
        }
    }

    /// Build a config with the given tools and tool choice.
    fn build_config(&self, tools: &[Tool], tool_choice: ToolChoice) -> General {
        self.config
            .clone()
            .with_tools(tools.to_vec())
            .with_tool_choice(tool_choice)
    }

    /// Convert a [`General`] config to the provider's [`ChatConfig`](LLM::ChatConfig).
    fn provider_cfg(&self, general: General) -> <H::Provider as LLM>::ChatConfig {
        general.into()
    }

    /// Internal send loop (non-streaming).
    async fn send_inner(
        &self,
        agent: &str,
        session: &mut Session,
        message: Message,
    ) -> Result<Response> {
        let agent_config = self
            .agents
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not registered"))?;
        let tools = self.resolve(&agent_config.tools);
        let mut tool_choice = ToolChoice::Auto;
        session.messages.push(message);
        self.maybe_compact(agent, session).await;

        for _ in 0..MAX_TOOL_CALLS {
            let messages = self.api_messages(agent, session).await;
            let cfg = self.build_config(&tools, tool_choice.clone());
            let pcfg = self.provider_cfg(cfg);
            let response = self.provider.send(&pcfg, &messages).await?;
            let Some(message) = response.message() else {
                return Ok(response);
            };

            if message.tool_calls.is_empty() {
                session.messages.push(message);
                return Ok(response);
            }

            let result = self.dispatch(&message.tool_calls).await;
            session.messages.push(message);
            session.messages.extend(result);
            tool_choice = ToolChoice::None;
        }

        anyhow::bail!("max tool calls reached");
    }

    /// Stateless send: run the LLM send loop with externally managed history.
    ///
    /// Unlike [`send_to`](Self::send_to), this takes `&self` (no mutation) and
    /// accepts the caller's message history directly. No session management or
    /// compaction — the caller owns the history.
    pub async fn send_stateless(
        &self,
        agent: &str,
        messages: &mut Vec<Message>,
        content: &str,
    ) -> Result<String> {
        let agent_config = self
            .agents
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("agent '{agent}' not registered"))?;
        let tools = self.resolve(&agent_config.tools);
        let mut tool_choice = ToolChoice::Auto;
        messages.push(Message::user(content));

        for _ in 0..MAX_TOOL_CALLS {
            let api_msgs = self.api_messages_from(agent, messages).await;
            let cfg = self.build_config(&tools, tool_choice.clone());
            let pcfg = self.provider_cfg(cfg);
            let response = self.provider.send(&pcfg, &api_msgs).await?;
            let Some(message) = response.message() else {
                return Ok(response.content().cloned().unwrap_or_default());
            };

            if message.tool_calls.is_empty() {
                let result = message.content.clone();
                messages.push(message);
                return Ok(result);
            }

            let result = self.dispatch(&message.tool_calls).await;
            messages.push(message);
            messages.extend(result);
            tool_choice = ToolChoice::None;
        }

        anyhow::bail!("max tool calls reached");
    }
}

//! Env — the embeddable engine environment.
//!
//! [`Env`] orchestrates registered [`Hook`] implementations (tools and
//! subsystems) and provides scope enforcement for dispatch. Tool
//! handlers are registered dynamically at startup via `register_hook`.

use crate::{Hook, host::Host};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tokio::sync::{Mutex, oneshot};
use wcore::{
    AgentConfig, AgentEvent, ToolDispatch, ToolDispatcher, ToolFuture, model::HistoryEntry,
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
pub type ConversationCwds = Arc<Mutex<HashMap<u64, PathBuf>>>;

/// Pending ask_user oneshots (shared with ask_user handler and protocol layer).
pub type PendingAsks = Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>;

/// Late-bindable sink for `agent:{name}:done` event publishes.
pub type EventSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub struct Env<H: Host> {
    pub(crate) cwd: PathBuf,
    pub scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
    pub(crate) agent_descriptions: RwLock<BTreeMap<String, String>>,
    /// Per-conversation CWD overrides (shared with OsHook + DelegateHook).
    pub conversation_cwds: ConversationCwds,
    /// Pending ask_user replies (shared with AskUserHook).
    pub pending_asks: PendingAsks,
    /// Host providing server-specific functionality.
    pub host: H,
    /// Registered hooks keyed by subsystem name.
    hooks: BTreeMap<String, Arc<dyn Hook>>,
    /// Tool name → owning hook for O(log n) dispatch.
    dispatch_map: BTreeMap<String, Arc<dyn Hook>>,
    /// Late-bound sink for publishing `agent:{name}:done` events.
    event_sink: RwLock<Option<EventSink>>,
}

impl<H: Host> Env<H> {
    /// Create a new Env with the given backends.
    pub fn new(
        cwd: PathBuf,
        host: H,
        scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
        conversation_cwds: ConversationCwds,
        pending_asks: PendingAsks,
    ) -> Self {
        Self {
            cwd,
            scopes,
            agent_descriptions: RwLock::new(BTreeMap::new()),
            conversation_cwds,
            pending_asks,
            host,
            hooks: BTreeMap::new(),
            dispatch_map: BTreeMap::new(),
            event_sink: RwLock::new(None),
        }
    }

    /// Register a hook (tool subsystem) by name.
    ///
    /// The hook's `schema()` is called to populate the dispatch map so
    /// tool calls route to this hook in O(log n).
    pub fn register_hook(&mut self, name: impl Into<String>, hook: Arc<dyn Hook>) {
        for tool in hook.schema() {
            self.dispatch_map
                .insert(tool.function.name.clone(), hook.clone());
        }
        self.hooks.insert(name.into(), hook);
    }

    /// Install the late-bound [`EventSink`] used by `on_event` to publish
    /// `agent:{name}:done` events.
    pub fn set_event_sink(&self, sink: EventSink) {
        *self.event_sink.write().expect("event_sink lock poisoned") = Some(sink);
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
        if MEMORY_TOOLS
            .iter()
            .any(|&t| self.dispatch_map.contains_key(t))
        {
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

    /// Route a tool call by name to the appropriate handler.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        sender: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        // Scope enforcement: reject tools not in the agent's whitelist.
        {
            let scopes = self.scopes.read().expect("scopes lock poisoned");
            if let Some(scope) = scopes.get(agent)
                && !scope.tools.is_empty()
                && !scope.tools.iter().any(|t| t.as_str() == name)
            {
                return Err(format!("tool not available: {name}"));
            }
        }

        let call = ToolDispatch {
            args: args.to_owned(),
            agent: agent.to_owned(),
            sender: sender.to_owned(),
            conversation_id,
        };

        if let Some(hook) = self.dispatch_map.get(name) {
            if let Some(fut) = hook.dispatch(name, call) {
                return fut.await;
            }
        }

        Err(format!("tool not registered: {name}"))
    }
}

impl<H: Host + 'static> ToolDispatcher for Env<H> {
    fn dispatch<'a>(
        &'a self,
        name: &'a str,
        args: &'a str,
        agent: &'a str,
        sender: &'a str,
        conversation_id: Option<u64>,
    ) -> ToolFuture<'a> {
        Box::pin(self.dispatch_tool(name, args, agent, sender, conversation_id))
    }
}

impl<H: Host + 'static> Hook for Env<H> {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        for hook in self.hooks.values() {
            if let Some(ref prompt) = hook.system_prompt() {
                config.system_prompt.push_str(prompt);
            }
        }
        self.apply_scope(&mut config);
        config
    }

    fn preprocess(&self, agent: &str, content: &str) -> Option<String> {
        for hook in self.hooks.values() {
            if let Some(result) = hook.preprocess(agent, content) {
                return Some(result);
            }
        }
        None
    }

    fn on_before_run(
        &self,
        agent: &str,
        conversation_id: u64,
        history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        let mut injected = Vec::new();

        // Agent member descriptions (delegate coordination).
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
                injected.push(HistoryEntry::user(block).auto_injected());
            }
        }

        // Hook-provided before_run.
        for hook in self.hooks.values() {
            injected.extend(hook.on_before_run(agent, conversation_id, history));
        }

        // Layered instructions (Crab.md).
        let cwd = self
            .conversation_cwds
            .try_lock()
            .ok()
            .and_then(|m| m.get(&conversation_id).cloned())
            .unwrap_or_else(|| self.cwd.clone());
        if let Some(instructions) = self.host.discover_instructions(&cwd) {
            injected.push(
                HistoryEntry::user(format!("<instructions>\n{instructions}\n</instructions>"))
                    .auto_injected(),
            );
        }

        // Guest agent framing.
        if history.iter().any(|e| !e.agent.is_empty()) {
            injected.push(
                HistoryEntry::user(
                    "Messages wrapped in <from agent=\"...\"> tags are from guest agents \
                     who were consulted in this conversation. Continue responding as yourself."
                        .to_string(),
                )
                .auto_injected(),
            );
        }
        injected
    }

    fn on_event(&self, agent: &str, conversation_id: u64, event: &AgentEvent) {
        for hook in self.hooks.values() {
            hook.on_event(agent, conversation_id, event);
        }

        self.host.on_agent_event(agent, conversation_id, event);

        if let AgentEvent::Done(response) = event
            && let Some(sink) = self
                .event_sink
                .read()
                .expect("event_sink lock poisoned")
                .clone()
        {
            let source = format!("agent:{agent}:done");
            let payload = response.final_response.clone().unwrap_or_default();
            sink(&source, &payload);
        }
    }
}

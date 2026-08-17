//! Harnesses — lifecycle callbacks and tool dispatch for subsystems.
//!
//! Each subsystem implements [`Harness`] to participate in the runtime
//! lifecycle: provide schemas, inject context before runs, observe events,
//! preprocess messages, and dispatch tool calls. [`Hooks`] is the composite
//! the runtime `Env` actually sees, and the built-in harnesses it aggregates
//! — memory and MCP — live alongside it.

use crate::{AgentEvent, ToolDispatch, ToolFuture};
use crabllm_core::Tool;
use parking_lot::RwLock;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use storage::AgentConfig;

pub use mcp::McpHook;
pub use memory::{Memory, MemoryHook};

pub mod mcp;
pub mod memory;

/// A pluggable subsystem that participates in the agent lifecycle.
///
/// All methods have default no-op implementations so subsystems only
/// override what they need.
pub trait Harness: Send + Sync {
    /// Tool schemas this hook provides.
    fn schema(&self) -> Vec<Tool> {
        vec![]
    }

    /// When to reach for this hook's tools, and how they go together.
    ///
    /// A declaration rather than a lifecycle callback: it answers a question
    /// no single tool's `description` can, because it is about choosing
    /// between them. It is in context from the first turn — the model has to
    /// have it *before* it decides — so it is paid on every turn and belongs
    /// at that altitude. Anything longer than a few lines is a skill.
    fn usage(&self) -> Option<String> {
        None
    }

    /// Called by `Runtime::add_agent()` before building the `Agent`.
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        config
    }

    /// Called just before an agent is inserted into the runtime registry
    /// (via `upsert_agent`). Hooks that track per-agent state (e.g. scopes,
    /// descriptions) should record it here; the ordering guarantees that by
    /// the time the agent is visible via `Runtime::agent()`, hook state is
    /// already in place.
    fn on_register_agent(&self, _name: &str, _config: &AgentConfig) {}

    /// Called after an agent is removed from the runtime registry. Hooks
    /// should drop any per-agent state they own. Symmetric to
    /// `on_register_agent`: once the agent is invisible, hook state is
    /// cleaned up.
    fn on_unregister_agent(&self, _name: &str) {}

    /// Called by Runtime after each agent step during execution.
    fn on_event(&self, _agent: &str, _conversation_id: u64, _event: &AgentEvent) {}

    /// Preprocess user content before it becomes a message.
    /// Return `Some(modified)` to transform, `None` to pass through.
    fn preprocess(&self, _agent: &str, _content: &str) -> Option<String> {
        None
    }

    /// Tool schemas for one stream that opted into the named sub-hooks, merged
    /// into the request as `extra_tools`. Only the composite `Hooks` has any.
    fn scoped_schema(&self, _names: &[String]) -> Vec<Tool> {
        vec![]
    }

    /// Tools to include when building a scoped agent's whitelist, plus an
    /// optional scope prompt line (e.g. `"skills: foo, bar"`).
    ///
    /// Default: include all tools from `schema()` unconditionally, no
    /// scope line. Override to gate inclusion on agent config fields.
    fn scoped_tools(&self, _config: &AgentConfig) -> (Vec<String>, Option<String>) {
        let tools = self
            .schema()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        (tools, None)
    }

    /// Dispatch a tool call by name. Return `None` if this hook doesn't
    /// own the tool — Env will try the next hook or the legacy entries.
    fn dispatch<'a>(&'a self, _name: &'a str, _call: ToolDispatch) -> Option<ToolFuture<'a>> {
        None
    }
}

/// No-op Harness for tests.
impl Harness for () {}

/// Per-agent scope for dispatch enforcement. An empty vec is unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub tools: Vec<String>,
}

/// Late-bindable sink for `agent:{name}:done` event publishes.
pub type EventSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Aggregates all sub-hooks behind a single `Harness` impl.
pub struct Hooks {
    pub scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
    hooks: BTreeMap<String, Arc<dyn Harness>>,
    /// Dispatchable but never advertised ambiently, so a surface's own tools
    /// can't leak into ordinary chat or unattended heartbeats.
    scoped: BTreeMap<String, Arc<dyn Harness>>,
    /// Tool names owned by `scoped`, which dispatch lets past the per-agent
    /// whitelist — declaring one is already the gate.
    scoped_names: BTreeSet<String>,
    dispatch_map: BTreeMap<String, Arc<dyn Harness>>,
    event_sink: RwLock<Option<EventSink>>,
}

impl Hooks {
    pub fn new(scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>) -> Self {
        Self {
            scopes,
            hooks: BTreeMap::new(),
            scoped: BTreeMap::new(),
            scoped_names: BTreeSet::new(),
            dispatch_map: BTreeMap::new(),
            event_sink: RwLock::new(None),
        }
    }

    /// Register a sub-hook by name.
    pub fn register_hook(&mut self, name: impl Into<String>, hook: Arc<dyn Harness>) {
        for tool in hook.schema() {
            self.dispatch_map
                .insert(tool.function.name.clone(), hook.clone());
        }
        self.hooks.insert(name.into(), hook);
    }

    /// Register a sub-hook whose tools a stream must opt into by name with
    /// [`Hooks::scoped_schema`]; see the `scoped` field.
    pub fn register_scoped(&mut self, name: impl Into<String>, hook: Arc<dyn Harness>) {
        for tool in hook.schema() {
            self.scoped_names.insert(tool.function.name.clone());
            self.dispatch_map
                .insert(tool.function.name.clone(), hook.clone());
        }
        self.scoped.insert(name.into(), hook);
    }

    /// Install the late-bound event sink for `agent:{name}:done` events.
    pub fn set_event_sink(&self, sink: EventSink) {
        *self.event_sink.write() = Some(sink);
    }

    /// Apply scoped tool whitelist and scope prompt for sub-agents.
    fn apply_scope(&self, config: &mut AgentConfig) {
        let has_scoping = !config.skills.is_empty() || !config.mcps.is_empty();
        // Skills allowlist + MCP declarations both produce a tool whitelist
        // that the dispatcher enforces. No declarations → no whitelist needed.
        if !has_scoping {
            return;
        }

        let mut whitelist = Vec::new();
        let mut scope_lines = Vec::new();
        for hook in self.hooks.values() {
            let (tools, line) = hook.scoped_tools(config);
            whitelist.extend(tools);
            if let Some(line) = line {
                scope_lines.push(line);
            }
        }

        if !scope_lines.is_empty() {
            let scope_block = format!("\n\n<scope>\n{}\n</scope>", scope_lines.join("\n"));
            config.description.push_str(&scope_block);
        }

        config.tools = whitelist;
    }
}

impl Harness for Hooks {
    fn schema(&self) -> Vec<Tool> {
        self.hooks.values().flat_map(|h| h.schema()).collect()
    }

    /// Unknown names are skipped, so the default — nothing declared — exposes
    /// no scoped tools at all.
    fn scoped_schema(&self, names: &[String]) -> Vec<Tool> {
        names
            .iter()
            .filter_map(|name| self.scoped.get(name))
            .flat_map(|hook| hook.schema())
            .collect()
    }

    fn usage(&self) -> Option<String> {
        let mut usage = String::new();
        for hook in self.hooks.values() {
            if let Some(ref declared) = hook.usage() {
                usage.push_str(declared);
            }
        }
        if usage.is_empty() { None } else { Some(usage) }
    }

    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        // The description is used as written. Framing it — "You are X." — was
        // the daemon supplying prose nobody asked for, and an agent that
        // cannot say who it is in its own description will not be rescued by
        // a sentence we prepend.
        if let Some(ref usage) = self.usage() {
            config.description.push_str(usage);
        }
        self.apply_scope(&mut config);
        config
    }

    fn on_register_agent(&self, name: &str, config: &AgentConfig) {
        self.scopes.write().insert(
            name.to_owned(),
            AgentScope {
                tools: config.tools.clone(),
            },
        );
        for hook in self.hooks.values() {
            hook.on_register_agent(name, config);
        }
    }

    fn on_unregister_agent(&self, name: &str) {
        self.scopes.write().remove(name);
        for hook in self.hooks.values() {
            hook.on_unregister_agent(name);
        }
    }

    fn on_event(&self, agent: &str, conversation_id: u64, event: &AgentEvent) {
        for hook in self.hooks.values() {
            hook.on_event(agent, conversation_id, event);
        }

        if let AgentEvent::Done(response) = event
            && let Some(sink) = self.event_sink.read().clone()
        {
            let source = format!("agent:{agent}:done");
            let payload = response.final_response.clone().unwrap_or_default();
            sink(&source, &payload);
        }
    }

    fn preprocess(&self, agent: &str, content: &str) -> Option<String> {
        for hook in self.hooks.values() {
            if let Some(result) = hook.preprocess(agent, content) {
                return Some(result);
            }
        }
        None
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        // Scoped tools skip the whitelist — declaring one is already the gate,
        // and they are never in the persistent whitelist to begin with.
        if !self.scoped_names.contains(name) {
            let scopes = self.scopes.read();
            if let Some(scope) = scopes.get(&call.agent)
                && !scope.tools.is_empty()
                && !scope.tools.iter().any(|t| t.as_str() == name)
            {
                return Some(Box::pin(async move {
                    Err(format!("tool not available: {name}"))
                }));
            }
        }

        if let Some(hook) = self.dispatch_map.get(name) {
            return hook.dispatch(name, call);
        }

        None
    }
}

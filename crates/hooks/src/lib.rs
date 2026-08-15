//! Reusable hook implementations and composition for the Crabtalk runtime.
//!
//! `Hooks` is the single `Hook` the runtime `Env` sees — it owns the
//! sub-hooks registered into it, the dispatch map, per-agent scope
//! enforcement, agent descriptions, and the late-bound event sink.

use crabllm_core::Tool;
use parking_lot::RwLock;
use runtime::Hook;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use wcore::{AgentConfig, AgentEvent, ToolDispatch, ToolFuture};

#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "memory")]
pub mod memory;
#[cfg(feature = "skill")]
pub mod skill;
#[cfg(feature = "memory")]
mod utils;

#[cfg(feature = "mcp")]
pub use mcp::McpHook;
#[cfg(feature = "memory")]
pub use memory::{DEFAULT_SOUL, Memory, MemoryHook};
#[cfg(feature = "skill")]
pub use skill::handler::SkillHook;
#[cfg(feature = "memory")]
pub use utils::default_crab;

/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}

/// Late-bindable sink for `agent:{name}:done` event publishes.
pub type EventSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Aggregates all sub-hooks behind a single `Hook` impl.
pub struct Hooks {
    pub scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
    hooks: BTreeMap<String, Arc<dyn Hook>>,
    /// Dispatchable but never advertised ambiently, so a surface's own tools
    /// can't leak into ordinary chat or unattended heartbeats.
    scoped: BTreeMap<String, Arc<dyn Hook>>,
    /// Tool names owned by `scoped`, which dispatch lets past the per-agent
    /// whitelist — declaring one is already the gate.
    scoped_names: BTreeSet<String>,
    dispatch_map: BTreeMap<String, Arc<dyn Hook>>,
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
    pub fn register_hook(&mut self, name: impl Into<String>, hook: Arc<dyn Hook>) {
        for tool in hook.schema() {
            self.dispatch_map
                .insert(tool.function.name.clone(), hook.clone());
        }
        self.hooks.insert(name.into(), hook);
    }

    /// Register a sub-hook whose tools a stream must opt into by name with
    /// [`Hooks::scoped_schema`]; see the `scoped` field.
    pub fn register_scoped(&mut self, name: impl Into<String>, hook: Arc<dyn Hook>) {
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
            config.system_prompt.push_str(&scope_block);
        }

        config.tools = whitelist;
    }
}

impl Hook for Hooks {
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

    fn system_prompt(&self) -> Option<String> {
        let mut prompt = String::new();
        for hook in self.hooks.values() {
            if let Some(ref s) = hook.system_prompt() {
                prompt.push_str(s);
            }
        }
        if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        }
    }

    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        // A store that persists only a description composes the identity line
        // here; one that persists a full prompt already filled this in, so
        // seeding only when empty leaves it untouched.
        if config.system_prompt.is_empty() && !config.description.is_empty() {
            config.system_prompt = format!("You are {}.\n\n{}", config.name, config.description);
        }
        if let Some(ref prompt) = self.system_prompt() {
            config.system_prompt.push_str(prompt);
        }
        self.apply_scope(&mut config);
        config
    }

    fn on_register_agent(&self, name: &str, config: &AgentConfig) {
        self.scopes.write().insert(
            name.to_owned(),
            AgentScope {
                tools: config.tools.clone(),
                skills: config.skills.clone(),
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

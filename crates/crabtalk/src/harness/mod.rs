//! The harness registry — the composite harness the runtime sees, plus
//! the built-in MCP and memory subsystems.

mod mcp;
mod memory;

use crabllm_core::Tool;
use crabtalk_berm::BermHarness;
use parking_lot::RwLock;
use runtime::{AgentEvent, Harness, ToolDispatch, ToolFuture};
use std::{collections::BTreeMap, sync::Arc};
use store::{AgentConfig, AgentId, interface::Memory};

pub use mcp::{Mcp, McpHarness};
pub use memory::MemoryHarness;

/// Per-agent scope for dispatch enforcement. An empty vec is unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub tools: Vec<String>,
}

/// Late-bindable sink for `agent:{name}:done` event publishes.
pub type EventSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Aggregates the built-in harnesses and the consumer chain behind a
/// single `Harness` impl — the one harness the runtime sees.
pub struct HarnessRegistry<M: Memory + 'static> {
    scopes: Arc<RwLock<BTreeMap<AgentId, AgentScope>>>,
    berm: Option<Arc<BermHarness>>,
    pub mcp: Arc<McpHarness>,
    memory: Arc<MemoryHarness<M>>,
    /// Consumer harnesses, in declaration order.
    hooks: Vec<Arc<dyn Harness>>,
    dispatch_map: BTreeMap<String, Arc<dyn Harness>>,
    event_sink: RwLock<Option<EventSink>>,
}

impl<M: Memory + 'static> HarnessRegistry<M> {
    pub fn new(
        scopes: Arc<RwLock<BTreeMap<AgentId, AgentScope>>>,
        berm: Option<Arc<BermHarness>>,
        mcp: Arc<McpHarness>,
        memory: Arc<MemoryHarness<M>>,
    ) -> Result<Self, String> {
        let mut registry = Self {
            scopes,
            berm,
            mcp,
            memory,
            hooks: Vec::new(),
            dispatch_map: BTreeMap::new(),
            event_sink: RwLock::new(None),
        };
        let members = registry.members();
        for member in members {
            registry.index(&member)?;
        }
        Ok(registry)
    }

    /// Register a consumer harness. Fails on a tool name already owned
    /// by a builtin or an earlier registration.
    pub fn register(&mut self, harness: Arc<dyn Harness>) -> Result<(), String> {
        self.index(&harness)?;
        self.hooks.push(harness);
        Ok(())
    }

    /// Install the late-bound event sink for `agent:{name}:done` events.
    pub fn set_event_sink(&self, sink: EventSink) {
        *self.event_sink.write() = Some(sink);
    }

    /// Index one member's tools into the dispatch map, erroring on a
    /// name collision.
    fn index(&mut self, member: &Arc<dyn Harness>) -> Result<(), String> {
        let names: Vec<String> = member
            .schema()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        if let Some(name) = names
            .iter()
            .find(|name| self.dispatch_map.contains_key(*name))
        {
            return Err(format!("duplicate tool: {name}"));
        }
        for name in names {
            self.dispatch_map.insert(name, member.clone());
        }
        Ok(())
    }

    /// Members in composition order: builtin fields first, then the
    /// consumer chain in declaration order.
    fn members(&self) -> Vec<Arc<dyn Harness>> {
        let mut out: Vec<Arc<dyn Harness>> = Vec::new();
        if let Some(berm) = &self.berm {
            out.push(berm.clone());
        }
        out.push(self.mcp.clone());
        out.push(self.memory.clone());
        out.extend(self.hooks.iter().cloned());
        out
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
        for member in self.members() {
            let (tools, line) = member.scoped_tools(config);
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

impl<M: Memory + 'static> Harness for HarnessRegistry<M> {
    fn schema(&self) -> Vec<Tool> {
        self.members().iter().flat_map(|h| h.schema()).collect()
    }

    fn usage(&self) -> Option<String> {
        let mut usage = String::new();
        for member in self.members() {
            if let Some(ref declared) = member.usage() {
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

    fn on_resolve_agent(&self, id: &AgentId, config: &AgentConfig) {
        self.scopes.write().insert(
            *id,
            AgentScope {
                tools: config.tools.clone(),
            },
        );
        for member in self.members() {
            member.on_resolve_agent(id, config);
        }
    }

    fn on_forget_agent(&self, id: &AgentId) {
        self.scopes.write().remove(id);
        for member in self.members() {
            member.on_forget_agent(id);
        }
    }

    fn on_event(&self, agent: &AgentId, session_id: u64, event: &AgentEvent) {
        for member in self.members() {
            member.on_event(agent, session_id, event);
        }

        if let AgentEvent::Done(response) = event
            && let Some(sink) = self.event_sink.read().clone()
        {
            let source = format!("agent:{agent}:done");
            let payload = response.final_response.clone().unwrap_or_default();
            sink(&source, &payload);
        }
    }

    fn preprocess(&self, agent: &AgentId, content: &str) -> Option<String> {
        for member in self.members() {
            if let Some(result) = member.preprocess(agent, content) {
                return Some(result);
            }
        }
        None
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        let scopes = self.scopes.read();
        if let Some(scope) = scopes.get(&call.agent)
            && !scope.tools.is_empty()
            && !scope.tools.iter().any(|t| t.as_str() == name)
        {
            return Some(Box::pin(async move {
                Err(format!("tool not available: {name}"))
            }));
        }

        if let Some(hook) = self.dispatch_map.get(name) {
            return hook.dispatch(name, call);
        }

        None
    }
}

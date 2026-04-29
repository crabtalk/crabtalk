//! DaemonHook — composite hook aggregating all sub-hooks with shared state.
//!
//! This is the single Hook the runtime Env sees. It owns all sub-hooks
//! (os, memory, skill, delegate, ask_user, mcp), the dispatch map, scope
//! enforcement, agent descriptions, and the event sink.

use parking_lot::RwLock;
use runtime::Hook;
use std::{collections::BTreeMap, sync::Arc};
use wcore::{AgentConfig, AgentEvent, ToolDispatch, ToolFuture};

/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub tools: Vec<String>,
    pub skills: Vec<String>,
}

/// Late-bindable sink for `agent:{name}:done` event publishes.
pub type EventSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Composite hook aggregating all node sub-hooks.
pub struct DaemonHook {
    pub scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
    agent_descriptions: RwLock<BTreeMap<String, String>>,
    hooks: BTreeMap<String, Arc<dyn Hook>>,
    dispatch_map: BTreeMap<String, Arc<dyn Hook>>,
    event_sink: RwLock<Option<EventSink>>,
}

impl DaemonHook {
    pub fn new(scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>) -> Self {
        Self {
            scopes,
            agent_descriptions: RwLock::new(BTreeMap::new()),
            hooks: BTreeMap::new(),
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

impl Hook for DaemonHook {
    fn schema(&self) -> Vec<crabllm_core::Tool> {
        self.hooks.values().flat_map(|h| h.schema()).collect()
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
        if let Some(ref prompt) = self.system_prompt() {
            config.system_prompt.push_str(prompt);
        }
        // Peer-agents block — names and descriptions of every other
        // registered agent, for the `delegate` tool to target. Built
        // once at agent-build time; later registry mutations only
        // appear after the agent is rebuilt (re-upserted or
        // daemon-reloaded).
        let descriptions = self.agent_descriptions.read();
        let peers: Vec<_> = descriptions
            .iter()
            .filter(|(name, _)| name.as_str() != config.name)
            .collect();
        if !peers.is_empty() {
            config.system_prompt.push_str("\n\n<agents>\n");
            for (name, desc) in peers {
                config
                    .system_prompt
                    .push_str(&format!("- {name}: {desc}\n"));
            }
            config.system_prompt.push_str("</agents>");
        }
        self.apply_scope(&mut config);
        config
    }

    fn on_register_agent(&self, name: &str, config: &AgentConfig) {
        if name != wcore::paths::DEFAULT_AGENT && !config.description.is_empty() {
            self.agent_descriptions
                .write()
                .insert(name.to_owned(), config.description.clone());
        }
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
        self.agent_descriptions.write().remove(name);
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
        // Scope enforcement.
        {
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

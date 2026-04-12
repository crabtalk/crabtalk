//! NodeHook — composite hook aggregating all sub-hooks with shared state.
//!
//! This is the single Hook the runtime Env sees. It owns all sub-hooks
//! (os, memory, skill, delegate, ask_user, mcp), the dispatch map, scope
//! enforcement, agent descriptions, and the event sink.

use crate::hooks::os::ApprovalTx;
use runtime::Hook;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tokio::sync::{Mutex, oneshot};
use wcore::{AgentConfig, AgentEvent, ToolDispatch, ToolFuture, model::HistoryEntry};

/// Per-conversation working directory overrides.
pub type ConversationCwds = Arc<Mutex<HashMap<u64, PathBuf>>>;

/// Pending ask_user oneshots (shared with AskUserHook and protocol layer).
pub type PendingAsks = Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>;

/// Per-agent scope for dispatch enforcement. Empty vecs = unrestricted.
#[derive(Default)]
pub struct AgentScope {
    pub tools: Vec<String>,
    pub members: Vec<String>,
    pub skills: Vec<String>,
    pub mcps: Vec<String>,
}

/// Late-bindable sink for `agent:{name}:done` event publishes.
pub type EventSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Base tools always included in every agent's whitelist.
const BASE_TOOLS: &[&str] = &["ask_user", "read", "edit"];
const SHELL_TOOLS: &[&str] = &["bash"];
const SKILL_TOOLS: &[&str] = &["skill"];
const MCP_TOOLS: &[&str] = &["mcp"];
const MEMORY_TOOLS: &[&str] = &["recall", "remember", "memory", "forget"];
const TASK_TOOLS: &[&str] = &["delegate"];

/// Composite hook aggregating all node sub-hooks.
pub struct NodeHook {
    pub scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
    agent_descriptions: RwLock<BTreeMap<String, String>>,
    hooks: BTreeMap<String, Arc<dyn Hook>>,
    dispatch_map: BTreeMap<String, Arc<dyn Hook>>,
    event_sink: RwLock<Option<EventSink>>,
    /// Per-conversation CWD overrides (shared with Host + OsHook + DelegateHook).
    pub conversation_cwds: ConversationCwds,
    /// Pending ask_user replies (shared with AskUserHook + protocol layer).
    pub pending_asks: PendingAsks,
    /// Bash approval sender (cloned into OsHook).
    pub approval_tx: ApprovalTx,
}

impl NodeHook {
    pub fn new(
        scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
        conversation_cwds: ConversationCwds,
        pending_asks: PendingAsks,
        approval_tx: ApprovalTx,
    ) -> Self {
        Self {
            scopes,
            agent_descriptions: RwLock::new(BTreeMap::new()),
            hooks: BTreeMap::new(),
            dispatch_map: BTreeMap::new(),
            event_sink: RwLock::new(None),
            conversation_cwds,
            pending_asks,
            approval_tx,
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

    /// Install the late-bound event sink for `agent:{name}:done` events.
    pub fn set_event_sink(&self, sink: EventSink) {
        *self.event_sink.write().expect("event_sink lock poisoned") = Some(sink);
    }

    /// Apply scoped tool whitelist and scope prompt for sub-agents.
    fn apply_scope(&self, config: &mut AgentConfig) {
        let has_scoping =
            !config.skills.is_empty() || !config.mcps.is_empty() || !config.members.is_empty();
        if !has_scoping {
            return;
        }

        let mut whitelist: Vec<String> = BASE_TOOLS.iter().map(|&s| s.to_owned()).collect();
        if SHELL_TOOLS
            .iter()
            .any(|&t| self.dispatch_map.contains_key(t))
        {
            for &t in SHELL_TOOLS {
                whitelist.push(t.to_owned());
            }
        }
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
}

impl Hook for NodeHook {
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
        self.apply_scope(&mut config);
        config
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

        for hook in self.hooks.values() {
            injected.extend(hook.on_before_run(agent, conversation_id, history));
        }

        injected
    }

    fn on_event(&self, agent: &str, conversation_id: u64, event: &AgentEvent) {
        for hook in self.hooks.values() {
            hook.on_event(agent, conversation_id, event);
        }

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
            let scopes = self.scopes.read().expect("scopes lock poisoned");
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

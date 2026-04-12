//! Env — thin wrapper combining a Host with a composite Hook.
//!
//! The runtime engine sees `Env` as its Hook and ToolDispatcher. Env
//! delegates tool dispatch and lifecycle callbacks to the inner Hook,
//! adding Host-specific behavior (instruction discovery, event
//! broadcasting).

use crate::{Hook, host::Host};
use std::sync::Arc;
use wcore::{
    AgentConfig, AgentEvent, ToolDispatch, ToolDispatcher, ToolFuture, model::HistoryEntry,
};

pub struct Env<H: Host> {
    /// Host providing server-specific functionality.
    pub host: H,
    /// The composite hook that aggregates all sub-hooks.
    pub hook: Arc<dyn Hook>,
}

impl<H: Host> Env<H> {
    pub fn new(host: H, hook: Arc<dyn Hook>) -> Self {
        Self { host, hook }
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
        let call = ToolDispatch {
            args: args.to_owned(),
            agent: agent.to_owned(),
            sender: sender.to_owned(),
            conversation_id,
        };

        match self.hook.dispatch(name, call) {
            Some(fut) => fut,
            None => Box::pin(async move { Err(format!("tool not registered: {name}")) }),
        }
    }
}

impl<H: Host + 'static> Hook for Env<H> {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        self.hook.on_build_agent(config)
    }

    fn preprocess(&self, agent: &str, content: &str) -> Option<String> {
        self.hook.preprocess(agent, content)
    }

    fn on_before_run(
        &self,
        agent: &str,
        conversation_id: u64,
        history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        let mut injected = self.hook.on_before_run(agent, conversation_id, history);

        // Layered instructions (Crab.md).
        let cwd = self.host.effective_cwd(conversation_id);
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
        self.hook.on_event(agent, conversation_id, event);
        self.host.on_agent_event(agent, conversation_id, event);
    }
}

//! Hook trait — lifecycle callbacks and tool dispatch for subsystems.
//!
//! Each tool/subsystem implements `Hook` to participate in the runtime
//! lifecycle: provide schemas, inject context before runs, observe
//! events, preprocess messages, and dispatch tool calls.

use crabllm_core::Tool;
use wcore::{AgentConfig, AgentEvent, ToolDispatch, ToolFuture};

/// A pluggable subsystem that participates in the agent lifecycle.
///
/// All methods have default no-op implementations so subsystems only
/// override what they need.
pub trait Hook: Send + Sync {
    /// Tool schemas this hook provides.
    fn schema(&self) -> Vec<Tool> {
        vec![]
    }

    /// System prompt fragment appended to agent configs at build time.
    fn system_prompt(&self) -> Option<String> {
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

/// No-op Hook for tests.
impl Hook for () {}

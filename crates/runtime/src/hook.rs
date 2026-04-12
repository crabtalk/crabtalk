//! Hook trait — lifecycle callbacks and tool dispatch for subsystems.
//!
//! Each tool/subsystem implements `Hook` to participate in the runtime
//! lifecycle: provide schemas, inject context before runs, observe
//! events, preprocess messages, and dispatch tool calls.

use crabllm_core::Tool;
use wcore::{AgentConfig, AgentEvent, ToolDispatch, ToolFuture, model::HistoryEntry};

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

    /// Inject context entries before each agent run.
    fn on_before_run(
        &self,
        _agent: &str,
        _conversation_id: u64,
        _history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        Vec::new()
    }

    /// Called by Runtime after each agent step during execution.
    fn on_event(&self, _agent: &str, _conversation_id: u64, _event: &AgentEvent) {}

    /// Preprocess user content before it becomes a message.
    /// Return `Some(modified)` to transform, `None` to pass through.
    fn preprocess(&self, _agent: &str, _content: &str) -> Option<String> {
        None
    }

    /// Dispatch a tool call by name. Return `None` if this hook doesn't
    /// own the tool — Env will try the next hook or the legacy entries.
    fn dispatch<'a>(&'a self, _name: &'a str, _call: ToolDispatch) -> Option<ToolFuture<'a>> {
        None
    }
}

/// No-op Hook for tests.
impl Hook for () {}

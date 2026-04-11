//! Hook trait — lifecycle callbacks for agent building, event observation,
//! and preprocessing.
//!
//! All hook crates implement this trait. The runtime calls these methods
//! at the appropriate lifecycle points. Non-persistence methods default
//! to no-ops so implementors only override what they need.

#[cfg(feature = "test-utils")]
use crate::agent::tool::{ToolDispatcher, ToolFuture};
use crate::{AgentConfig, AgentEvent, model::HistoryEntry};

/// Lifecycle callbacks for agent building, event observation, and
/// preprocessing.
///
/// Non-persistence methods default to no-ops so implementors only
/// override what they need.
pub trait Hook: Send + Sync {
    /// Called by `Runtime::add_agent()` before building the `Agent`.
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        config
    }

    /// Called by Runtime after each agent step during execution.
    fn on_event(&self, _agent: &str, _conversation_id: u64, _event: &AgentEvent) {}

    /// Called by Runtime to preprocess user content before it becomes a message.
    fn preprocess(&self, _agent: &str, content: &str) -> String {
        content.to_owned()
    }

    /// Called by Runtime before each agent run (send_to / stream_to).
    fn on_before_run(
        &self,
        _agent: &str,
        _conversation_id: u64,
        _history: &[HistoryEntry],
    ) -> Vec<HistoryEntry> {
        Vec::new()
    }
}

/// Trivial [`Hook`] for tests that don't need lifecycle customization.
#[cfg(feature = "test-utils")]
#[derive(Default)]
pub struct TestHook;

#[cfg(feature = "test-utils")]
impl Hook for TestHook {}

#[cfg(feature = "test-utils")]
impl ToolDispatcher for TestHook {
    fn dispatch<'a>(
        &'a self,
        name: &'a str,
        _args: &'a str,
        _agent: &'a str,
        _sender: &'a str,
        _conversation_id: Option<u64>,
    ) -> ToolFuture<'a> {
        let name = name.to_owned();
        Box::pin(async move { Err(format!("TestHook: tool '{name}' not dispatched")) })
    }
}

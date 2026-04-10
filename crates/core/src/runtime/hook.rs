//! Hook trait — lifecycle callbacks for agent building, event observation,
//! tool schema registration, and preprocessing.
//!
//! All hook crates implement this trait. The runtime calls these methods
//! at the appropriate lifecycle points. Non-persistence methods default
//! to no-ops so implementors only override what they need.

use crate::{AgentConfig, AgentEvent, agent::tool::ToolRegistry, model::HistoryEntry};
use std::future::Future;

/// Lifecycle callbacks for agent building, event observation, tool
/// registration, and preprocessing.
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

    /// Called by `Runtime::new()` to register tool schemas.
    fn on_register_tools(&self, _tools: &mut ToolRegistry) -> impl Future<Output = ()> + Send {
        async {}
    }

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

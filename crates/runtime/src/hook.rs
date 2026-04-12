//! Hook trait — lifecycle callbacks for agent building, event observation,
//! and preprocessing.

use wcore::{AgentConfig, AgentEvent, model::HistoryEntry};

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

//! Hook trait — lifecycle backend for agent building, event observation,
//! tool schema registration, and persistence.
//!
//! All hook crates implement this trait. [`Runtime`](crate) calls these
//! methods at the appropriate lifecycle points and reaches the
//! persistence backend through the [`Hook::Repos`] associated type.

use crate::{
    AgentConfig, AgentEvent, agent::tool::ToolRegistry, model::HistoryEntry, repos::Repos,
};
use std::future::Future;

/// Lifecycle backend for agent building, event observation, tool
/// registration, and persistence.
///
/// Implementors supply a concrete [`Repos`] type via the associated
/// item — the runtime reaches it through [`repos`](Self::repos) and
/// uses it for session persistence, memory entries, skill loading, and
/// agent storage. Non-persistence methods default to no-ops so
/// implementors only override what they need.
pub trait Hook: Send + Sync {
    /// Persistence backend this hook exposes to the runtime.
    type Repos: Repos;

    /// Shared handle to the persistence backend. Conversation
    /// persistence, session replay, and subsystem state all route
    /// reads and writes through here.
    fn repos(&self) -> &Self::Repos;

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

/// Trivial [`Hook`] backed by [`InMemoryRepos`](crate::repos::mem::InMemoryRepos).
/// Useful in tests that need a `Runtime` but don't care about persistence.
#[cfg(feature = "test-utils")]
#[derive(Default)]
pub struct TestHook {
    repos: crate::repos::mem::InMemoryRepos,
}

#[cfg(feature = "test-utils")]
impl TestHook {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "test-utils")]
impl Hook for TestHook {
    type Repos = crate::repos::mem::InMemoryRepos;

    fn repos(&self) -> &Self::Repos {
        &self.repos
    }
}

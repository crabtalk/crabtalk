//! Hook trait — lifecycle backend for agent building, event observation,
//! and tool schema registration.
//!
//! All hook crates implement this trait. [`Runtime`](crate) calls these
//! methods at the appropriate lifecycle points. `DaemonEnv` composes
//! multiple Hook implementations by delegating to each.

use crate::{AgentConfig, AgentEvent, agent::tool::ToolRegistry, model::Message};
use std::future::Future;

/// Lifecycle backend for agent building, event observation, and tool registration.
///
/// Default implementations are no-ops so implementors only override what they need.
pub trait Hook: Send + Sync {
    /// Called by `Runtime::add_agent()` before building the `Agent`.
    ///
    /// Enriches the agent config: appends skill instructions, injects memory
    /// into the system prompt, etc. The returned config is passed to `AgentBuilder`.
    ///
    /// Default: returns config unchanged.
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        config
    }

    /// Called by Runtime after each agent step during execution.
    ///
    /// Receives every `AgentEvent` produced during `send_to` and `stream_to`.
    /// Use for logging, metrics, persistence, or forwarding.
    ///
    /// Default: no-op.
    fn on_event(&self, _agent: &str, _conversation_id: u64, _event: &AgentEvent) {}

    /// Called by `Runtime::new()` to register tool schemas into the registry.
    ///
    /// Implementations call `tools.insert(tool)` with schema-only `Tool` values.
    /// No handlers or closures are stored — dispatch is handled by the daemon.
    ///
    /// Default: no-op async.
    fn on_register_tools(&self, _tools: &mut ToolRegistry) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Called by Runtime to preprocess user content before it becomes a message.
    ///
    /// Used to resolve slash commands (e.g. `/skill-name args` → skill body + args).
    /// Returns the transformed content string.
    ///
    /// Default: returns content unchanged.
    fn preprocess(&self, _agent: &str, content: &str) -> String {
        content.to_owned()
    }

    /// Called by Runtime before each agent run (send_to / stream_to).
    ///
    /// Receives the agent name, conversation ID, and conversation history
    /// (including the latest user message). Returns messages to inject
    /// before the user message for additional context (e.g. memory,
    /// per-session environment).
    ///
    /// Default: no injection.
    fn on_before_run(&self, _agent: &str, _conversation_id: u64, _history: &[Message]) -> Vec<Message> {
        Vec::new()
    }
}

impl Hook for () {}

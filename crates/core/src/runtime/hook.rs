//! Hook trait — lifecycle backend for agent building, event observation,
//! and tool schema registration.
//!
//! All hook crates implement this trait. [`Runtime`](crate) calls these
//! methods at the appropriate lifecycle points. `DaemonHook` composes
//! multiple Hook implementations by delegating to each.

use crate::{AgentConfig, AgentEvent, Memory, agent::tool::ToolRegistry};
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
    fn on_event(&self, _agent: &str, _event: &AgentEvent) {}

    /// Called by `Runtime::new()` to register tool schemas into the registry.
    ///
    /// Implementations call `tools.insert(tool)` with schema-only `Tool` values.
    /// No handlers or closures are stored — dispatch is handled by the daemon.
    ///
    /// Default: no-op async.
    fn on_register_tools(&self, _tools: &mut ToolRegistry) -> impl Future<Output = ()> + Send {
        async {}
    }
}

impl Hook for () {}

/// Blanket Hook impl for all Memory types that are Clone + 'static.
///
/// Injects compiled memory into the system prompt via `on_build_agent`
/// and registers `remember`/`recall` tool schemas via `on_register_tools`.
impl<M: Memory + Clone + 'static> Hook for M {
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        let compiled = self.compile();
        config.system_prompt = format!("{}\n\n{compiled}", config.system_prompt);
        config
    }

    fn on_register_tools(&self, registry: &mut ToolRegistry) -> impl Future<Output = ()> + Send {
        use crate::memory::tools;
        registry.insert(tools::remember_schema());
        registry.insert(tools::recall_schema());
        async {}
    }
}

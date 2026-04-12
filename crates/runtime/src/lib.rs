mod conv;
mod engine;
pub mod env;
pub mod hook;
pub mod host;

pub use conv::Conversation;
pub use engine::Runtime;
pub use env::{AgentScope, ConversationCwds, Env, EventSink, PendingAsks};
pub use hook::Hook;
pub use host::Host;
pub use wcore::{MemoryConfig, SystemConfig, TasksConfig};

use crabllm_core::Provider;
use wcore::{agent::ToolDispatcher, storage::Storage};

/// Configuration trait bundling the associated types for a runtime.
///
/// Each binary defines one `Config` impl that ties together the
/// concrete storage, LLM provider, and hook implementations.
pub trait Config: Send + Sync + 'static {
    /// Persistence backend (sessions, agents, memory, skills).
    type Storage: Storage;

    /// LLM provider for agent execution.
    type Provider: Provider + 'static;

    /// Lifecycle hook for agent building, events, and tool dispatch.
    type Hook: Hook + ToolDispatcher;
}

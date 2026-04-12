mod conversation;
mod engine;
pub mod env;
pub mod hook;
pub mod host;

pub use conversation::Conversation;
pub use engine::Runtime;
pub use env::{AgentScope, ConversationCwds, Env, EventSink, PendingAsks};
pub use hook::Hook;
pub use host::Host;
pub use wcore::{MemoryConfig, SystemConfig, TasksConfig};

use crabllm_core::Provider;
use wcore::storage::Storage;

/// Configuration trait bundling the associated types for a runtime.
///
/// Each binary defines one `Config` impl that ties together the
/// concrete storage, LLM provider, and host implementations.
pub trait Config: Send + Sync + 'static {
    /// Persistence backend (sessions, agents, memory, skills).
    type Storage: Storage;

    /// LLM provider for agent execution.
    type Provider: Provider + 'static;

    /// Server-specific host capabilities (event broadcasting, instruction
    /// discovery).
    type Host: Host + 'static;
}

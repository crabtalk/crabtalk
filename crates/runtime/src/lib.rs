mod conversation;
mod engine;
pub mod env;
pub mod hook;

pub use conversation::Conversation;
pub use engine::{Runtime, SharedMemory, SwitchOutcome};
pub use env::Env;
pub use hook::Hook;
pub use wcore::{MemoryConfig, SystemConfig, TasksConfig};

use crabllm_core::Provider;
use wcore::storage::Storage;

/// Configuration trait bundling the associated types for a runtime.
///
/// Each binary defines one `Config` impl that ties together the
/// concrete storage, LLM provider, and env implementations.
pub trait Config: Send + Sync + 'static {
    /// Persistence backend (sessions, agents, memory, skills).
    type Storage: Storage;

    /// LLM provider for agent execution.
    type Provider: Provider + 'static;

    /// Node environment — event broadcasting, instruction discovery,
    /// and composite hook for tool dispatch.
    type Env: Env + wcore::ToolDispatcher + 'static;
}

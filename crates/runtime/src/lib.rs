//! The engine, and the agent it runs.
//!
//! What an agent *does* lives here. What one *is* — its config, and everything
//! persisted about it — lives in `crabtalk-storage`, which this crate reads.

pub mod agent;
mod conversation;
mod engine;
pub mod env;
pub mod harness;
pub mod sessions;

pub use agent::{
    Agent, AgentBuilder, Model,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    tool::{
        AsTool, BeforeRunHook, ToolDispatch, ToolDispatcher, ToolEntry, ToolFuture, ToolHandler,
        ToolRegistry,
    },
};
pub use conversation::Conversation;
pub use engine::{Program, ProgramStep, Runtime, SharedMemory};
pub use env::Env;
pub use harness::Harness;

/// Opaque persistent handle to a conversation. Re-exported from the
/// storage trait so runtime callers don't need to speak the storage
/// layer's "session" vocabulary.
pub type ConversationHandle = storage::SessionHandle;

use crabllm_core::Provider;
use storage::Storage;

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
    type Env: Env + ToolDispatcher + 'static;
}

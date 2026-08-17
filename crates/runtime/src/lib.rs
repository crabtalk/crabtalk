//! The engine, and the agent it runs.
//!
//! What an agent *does* lives here. What one *is* — its config, and everything
//! persisted about it — lives in `crabtalk-storage`, which this crate reads.

pub mod agent;
mod engine;
pub mod env;
pub mod harness;
mod session;

pub use agent::{
    Agent, AgentBuilder, Model,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    tool::{
        AsTool, BeforeRunHook, ToolDispatch, ToolDispatcher, ToolEntry, ToolFuture, ToolHandler,
        ToolRegistry,
    },
};
pub use engine::{Runtime, SharedMemory};
pub use env::Env;
pub use harness::Harness;
pub use session::{Session, Sessions, SharedSession};

/// A session's persistent identity, re-exported so callers get it
/// without depending on `storage` directly. No longer aliased to a
/// runtime-local name — both layers say "session" now.
pub use storage::SessionHandle;

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

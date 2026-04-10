//! Runtime types — conversation, hook, and configuration traits.

use crate::repos::Storage;
use crabllm_core::Provider;

pub mod conversation;
pub mod hook;

pub use conversation::Conversation;
pub use hook::Hook;

/// Configuration trait bundling the associated types for a runtime.
///
/// Each binary defines one `Config` impl that ties together the
/// concrete storage, LLM provider, and hook implementations.
pub trait Config: Send + Sync + 'static {
    /// Persistence backend (sessions, agents, memory, skills).
    type Storage: Storage;

    /// LLM provider for agent execution.
    type Provider: Provider + 'static;

    /// Lifecycle hook for agent building, events, and tool registration.
    type Hook: Hook;
}

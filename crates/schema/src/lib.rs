//! Crabtalk agent library.
//!
//! What an agent *does*. What one *is* — its config, and everything persisted
//! about it — lives in `crabtalk-storage`, which this crate reads.

pub use agent::{
    Agent, AgentBuilder,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    tool::{
        BeforeRunHook, ToolDispatch, ToolDispatcher, ToolEntry, ToolFuture, ToolHandler,
        ToolRegistry,
    },
};
// Re-exported so the agent half reads its own vocabulary. Temporary: when
// this crate folds into `crabtalk-runtime`, callers import from storage.
pub use ::storage::*;

pub mod agent;
pub mod model;

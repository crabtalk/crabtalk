//! Walrus agent library.
//!
//! - [`Agent`]: Immutable agent definition with step/run/run_stream.
//! - [`AgentBuilder`]: Fluent construction with a model provider.
//! - [`AgentConfig`]: Serializable agent parameters.
//! - [`Session`]: Lightweight conversation history container.
//! - [`ToolRegistry`]: Schema-only tool store. No handlers or closures.
//! - [`ToolSender`] / [`ToolRequest`]: Agent-side tool dispatch primitives.
//! - [`Hook`]: Lifecycle backend for agent building, events, and tool registration.
//! - [`Runtime`]: Agent registry, session store, and hook orchestration.
//! - [`model`]: Unified LLM interface types and traits.
//! - Agent event types: [`AgentEvent`], [`AgentStep`], [`AgentResponse`], [`AgentStopReason`].

pub use agent::{
    Agent, AgentBuilder, AgentConfig, COMPACT_SENTINEL,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    parse_agent_md,
    tool::{ToolRegistry, ToolRequest, ToolSender},
};
pub use runtime::{Runtime, Session, hook::Hook};

mod agent;
pub mod model;
pub mod paths;
pub mod protocol;
mod runtime;
pub mod utils;

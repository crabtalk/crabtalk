//! Crabtalk agent library.
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
    Agent, AgentBuilder, AgentConfig,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    tool::{ToolRegistry, ToolRequest, ToolSender},
};
pub use config::{
    ApiStandard, ManifestConfig, McpServerConfig, PackageMeta, ProviderDef, ResolvedManifest,
    Setup, check_skill_conflicts, load_agents_dir, load_agents_dirs, repo_slug, resolve_manifests,
    scan_skill_names,
};
pub use runtime::{
    Runtime, Session, hook::Hook, session::find_latest_session, session::sender_slug,
};

pub mod agent;
pub mod config;
pub mod model;
pub mod paths;
pub mod protocol;
mod runtime;
pub mod utils;

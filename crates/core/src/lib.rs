//! Crabtalk agent library.
//!
//! - [`Agent`]: Immutable agent definition with step/run/run_stream.
//! - [`AgentBuilder`]: Fluent construction with a model provider.
//! - [`AgentConfig`]: Serializable agent parameters.
//! - [`Conversation`]: Lightweight conversation history container.
//! - [`ToolRegistry`]: Schema-only tool store. No handlers or closures.
//! - [`ToolDispatcher`]: Agent-side tool dispatch trait.
//! - [`Hook`]: Lifecycle backend for agent building, events, and tool registration.
//! - [`model`]: Unified LLM interface types and traits.
//! - Agent event types: [`AgentEvent`], [`AgentStep`], [`AgentResponse`], [`AgentStopReason`].

pub use agent::{
    Agent, AgentBuilder, AgentConfig, AgentId,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    tool::{
        BeforeRunHook, ToolDispatch, ToolDispatcher, ToolEntry, ToolFuture, ToolHandler,
        ToolRegistry,
    },
};
pub use config::{
    ApiStandard, ManifestConfig, McpServerConfig, MemoryConfig, NodeConfig, PackageMeta,
    ProviderDef, ResolvedManifest, Setup, SystemConfig, TasksConfig, check_skill_conflicts,
    external_source_name, filter_disabled_external, load_agents_dir, load_agents_dirs, repo_slug,
    resolve_manifests, scan_skill_names, validate_providers,
};
#[cfg(feature = "test-utils")]
pub use runtime::hook::TestHook;
pub use runtime::{
    Config, Conversation,
    conversation::{ArchiveSegment, ConversationMeta, EventLine, sender_slug},
    hook::Hook,
};

pub mod agent;
pub mod config;
pub mod model;
pub mod paths;
pub mod protocol;
pub mod repos;
mod runtime;
pub mod utils;

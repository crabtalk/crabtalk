//! Crabtalk agent library.
//!
//! - [`Agent`]: Immutable agent definition with step/run/run_stream.
//! - [`AgentBuilder`]: Fluent construction with a model provider.
//! - [`AgentConfig`]: Serializable agent parameters.
//! - [`ToolRegistry`]: Schema-only tool store. No handlers or closures.
//! - [`ToolDispatcher`]: Agent-side tool dispatch trait.
//! - [`model`]: Unified LLM interface types and traits.
//! - [`storage`]: Unified persistence trait and domain types.
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
    ApiStandard, BashConfig, DaemonConfig, ManifestConfig, McpServerConfig, MemoryConfig,
    PackageMeta, ProviderDef, ResolvedManifest, Setup, SystemConfig, TasksConfig,
    check_skill_conflicts, external_source_name, filter_disabled_external, load_agents_dir,
    load_agents_dirs, repo_slug, resolve_manifests, scan_skill_names, validate_providers,
};
pub use storage::{ConversationMeta, EventLine, sender_slug};

pub mod agent;
pub mod config;
pub mod model;
pub mod paths;
pub mod protocol;
pub mod storage;
#[cfg(feature = "testing")]
pub mod testing;
pub mod utils;

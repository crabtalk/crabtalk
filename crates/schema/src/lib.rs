//! Crabtalk agent library.

pub use agent::{
    Agent, AgentBuilder, AgentConfig, AgentId,
    event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason},
    tool::{
        BeforeRunHook, ToolDispatch, ToolDispatcher, ToolEntry, ToolFuture, ToolHandler,
        ToolRegistry,
    },
};
pub use config::{
    Config, HarnessConfig, HooksConfig, LlmConfig, McpServerConfig, MemoryConfig, TasksConfig,
};
pub use storage::{ConversationMeta, EventLine, sender_slug};

pub mod agent;
pub mod config;
pub mod model;
pub mod paths;
pub mod storage;

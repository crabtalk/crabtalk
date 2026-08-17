//! Persistence backends for Crabtalk.
//!
//! [`Storage`](crate::Storage) is declared in core; this crate
//! implements it. A backend is chosen at compile time through
//! `runtime::Config`'s `Storage` associated type, not by a feature — the
//! `sqlite` feature exists only so a consumer that wants the filesystem
//! doesn't build a SQL driver it will never call.
//!
//! [`FsStorage`] is the daemon's backend: TOML configs, markdown prompts,
//! and JSON session files under `~/.crabtalk/`.

/// Mutable settings file (daemon-owned, persisted under `local/`).
pub const SETTINGS_FILE: &str = "local/settings.toml";
/// Daemon-owned state directory.
pub const LOCAL_DIR: &str = "local";
/// Skills subdirectory.
pub const SKILLS_DIR: &str = "local/skills";

pub use agent::{AgentConfig, AgentId, DEFAULT_AGENT};
pub use config::{
    CONFIG_FILE, Config, HarnessConfig, HooksConfig, LlmConfig, McpConfig, McpServerConfig,
    MemoryConfig, TasksConfig,
};
pub use history::HistoryEntry;
pub use storage::{
    ConversationMeta, EventLine, SessionHandle, SessionSnapshot, SessionSummary, Storage,
    ToolCallTrace, sender_slug, validate_table_name,
};

pub mod agent;
pub mod backend;
pub mod config;
pub mod history;
pub mod storage;

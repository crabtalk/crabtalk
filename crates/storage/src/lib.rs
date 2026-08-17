//! Persistence backends for Crabtalk.
//!
//! [`Storage`](crate::Storage) is declared in core; this crate
//! implements it. A backend is chosen at compile time through
//! `runtime::Config`'s `Storage` associated type, not by a feature — the
//! `sqlite` feature exists only so a consumer that wants the filesystem
//! doesn't build a SQL driver it will never call.
//!
//! [`SqliteStorage`](backend::SqliteStorage) is what ships here. The cloud
//! runs postgres behind the same trait — crabtalk is daemon-first, so a
//! store that answers queries beats a directory a process walks.

/// Skills subdirectory.
pub const SKILLS_DIR: &str = "local/skills";

pub use agent::{AgentConfig, AgentId, DEFAULT_AGENT};
pub use config::{
    CONFIG_FILE, Config, HarnessConfig, HooksConfig, LlmConfig, McpConfig, McpServerConfig,
    MemoryConfig, TasksConfig,
};
pub use session::history::HistoryEntry;
pub use storage::{
    EventLine, SessionHandle, SessionMeta, SessionSnapshot, SessionSummary, Storage, ToolCallTrace,
    sender_slug, validate_table_name,
};

pub use session::{
    MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS, SearchOptions, SessionHit, WindowItem,
};

pub mod agent;
pub mod backend;
pub mod config;
pub mod session;
pub mod storage;

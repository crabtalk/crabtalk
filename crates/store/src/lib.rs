//! Persistence for Crabtalk, in three layers.
//!
//! [`KVStorage`] holds content, and the secondary indexes that find it —
//! an ordered lookup, a name resolution, a set membership are all just
//! more keys. [`TextSearch`] holds the one thing keys cannot answer:
//! ranked full-text.
//!
//! The [`interface`] traits are bounded on those two and carry their own
//! bodies, so implementing the primitives *is* implementing the
//! interfaces — there is no wrapper to build and nothing to wire.
//!
//! A backend implements five methods and gets everything above them for
//! free — which is what makes a different deployment a different
//! implementation rather than a rewrite of anything here.
//! `crabtalk-agent` is one such backend, over [`crabdb`].
//!
//! [`crabdb`]: https://docs.rs/crabtalk-crabdb

pub use agent::{AgentConfig, AgentId, DEFAULT_AGENT};
pub use config::{
    CONFIG_FILE, CacheConfig, Config, HarnessConfig, HooksConfig, LlmConfig, McpConfig,
    McpServerConfig, MemoryConfig, TasksConfig,
};
pub use interface::{
    Agents, Backend, Harnesses, Memory, MemoryEntry, Sessions, Skill, SkillSummary, Skills,
    Weights, validate_table_name,
};
pub use kv::{Column, KVStorage, Realm};
pub use session::{
    EventLine, HistoryEntry, MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS,
    SearchOptions, SessionHandle, SessionHit, SessionMeta, SessionSnapshot, ToolCallTrace,
    WindowItem, sender_slug,
};
pub use text::{TextHit, TextIndex, TextSearch};

pub mod agent;
pub mod config;
pub mod interface;
pub mod kv;
pub mod session;
pub mod text;

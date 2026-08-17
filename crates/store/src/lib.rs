//! Persistence for Crabtalk, in three layers.
//!
//! [`KVStorage`] holds content, and the secondary indexes that find it —
//! an ordered lookup, a name resolution, a set membership are all just
//! more keys. [`TextSearch`] holds the one thing keys cannot answer:
//! ranked full-text. [`Store`] builds every query the runtime asks for
//! on those two, and implements the [`interface`] traits.
//!
//! A backend implements eight methods and gets everything above them
//! for free — which is what makes a different deployment a different
//! implementation rather than a rewrite of anything here.
//! `crabtalk-agent` is one such backend: sqlite behind both.

pub use agent::{AgentConfig, AgentId, DEFAULT_AGENT};
pub use config::{
    CONFIG_FILE, Config, HarnessConfig, HooksConfig, LlmConfig, McpConfig, McpServerConfig,
    MemoryConfig, TasksConfig,
};
pub use interface::{Agents, Backend, Harnesses, Memory, Sessions, Skills};
pub use kv::{Column, KVStorage, MemoryDb, Tenant};
pub use memory::MemoryEntry;
pub use session::{
    EventLine, HistoryEntry, MAX_HITS_PER_QUERY, MAX_SNIPPET_BYTES, MAX_WINDOW_ITEMS,
    SearchOptions, SessionHandle, SessionHit, SessionMeta, SessionSnapshot, ToolCallTrace,
    WindowItem, sender_slug,
};
pub use skill::{Skill, SkillSummary};
pub use store::Store;
pub use text::{TextHit, TextIndex, TextSearch};
pub use utils::validate_table_name;

pub mod agent;
pub mod config;
pub mod interface;
pub mod kv;
pub mod memory;
pub mod session;
pub mod skill;
pub mod store;
pub mod text;
pub mod utils;

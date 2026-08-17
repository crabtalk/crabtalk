//! Persistence for Crabtalk, in three layers.
//!
//! [`KVStorage`] holds content addressed by a key the caller already
//! has. [`SqlIndex`] holds what a lookup needs to *find* that key, and
//! nothing else — so it is derived, rebuildable, and never the thing a
//! crash can corrupt. [`Store`] pairs them and implements the
//! [`interface`] traits the runtime programs against.
//!
//! Only the primitives are implemented per backend. Everything above
//! them is written once, which is what makes a different deployment a
//! different implementation rather than a rewrite of anything here.
//! `crabtalk-agent` is one such backend: sqlite behind both halves.

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
pub use sql::{MessageDoc, SqlIndex};
pub use store::Store;
pub use utils::validate_table_name;

pub mod agent;
pub mod config;
pub mod interface;
pub mod kv;
pub mod memory;
pub mod session;
pub mod skill;
pub mod sql;
pub mod store;
pub mod utils;

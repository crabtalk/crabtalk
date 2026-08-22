//! Persistence for Crabtalk, in two layers.
//!
//! The [`interface`] traits are what the runtime programs against, and
//! they name no storage: an agent, a session, a memory entry, a skill,
//! a harness image. [`TextSearch`] sits beside them for the one thing
//! keys cannot answer, ranked full-text.
//!
//! Below them, [`KVStorage`] is five methods — content, plus the
//! secondary indexes that find it, since an ordered lookup, a name
//! resolution and a set membership are all just more keys. Every
//! interface above is blanket-implemented over it, so a backend writes
//! those five and has a working daemon. `crabtalk-agent` is one such
//! backend, over [`crabdb`].
//!
//! A store whose engine already models sessions and agents takes the
//! other door: implement the interfaces directly, name no key, and skip
//! the KV entirely.
//!
//! [`crabdb`]: https://docs.rs/crabtalk-crabdb

pub use agent::{AgentConfig, AgentId, DEFAULT_AGENT};
pub use config::{
    CacheConfig, Config, HarnessConfig, HooksConfig, LlmConfig, McpConfig, McpServerConfig,
    MemoryConfig, Root, TasksConfig,
};
pub use interface::{
    Agents, Backend, EventSubscription, Harnesses, Memory, MemoryEntry, Sessions, Skill,
    SkillSummary, Skills, Subscriptions, Weights, validate_table_name,
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

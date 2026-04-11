//! Persistence traits and domain types.
//!
//! [`Storage`] is the unified persistence backend — one trait, one
//! implementation per backend.

pub mod memory;
pub mod sessions;
pub mod skills;
pub mod storage;

pub use memory::{MemoryEntry, slugify};
pub use sessions::{SessionHandle, SessionSnapshot, SessionSummary};
pub use skills::Skill;
pub use storage::Storage;

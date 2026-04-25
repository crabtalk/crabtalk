//! Standalone single-file memory system.
//!
//! `Memory` is a connection to a single db file (like SQLite): entries,
//! aliases, and an inverted index all live in one place. v1 is in-RAM only;
//! persistence and dump/load land in later phases.

pub mod bm25;
mod dump;
mod entry;
mod error;
mod file;
mod memory;
mod op;

pub use crate::{
    entry::{Entry, EntryId, EntryKind},
    error::{Error, Result},
    memory::{Memory, SearchHit},
    op::Op,
};

//! Re-export of [`MemoryEntry`] from core.
//!
//! The domain type lives in `wcore::repos::memory` alongside the
//! `MemoryRepo` trait. This module re-exports it for backward
//! compatibility within the runtime crate.

pub use wcore::repos::{MemoryEntry, slugify};

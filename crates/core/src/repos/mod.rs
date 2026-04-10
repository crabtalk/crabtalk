//! Domain-shaped repository traits for runtime persistence.
//!
//! Four traits — one per persistence aggregate:
//!
//! - [`MemoryRepo`] — memory entries + curated index.
//! - [`SkillRepo`] — skill discovery and loading.
//! - [`SessionRepo`] — conversation sessions (create, append, replay).
//! - [`AgentRepo`] — agent config + prompt persistence.
//!
//! The [`Repos`] composite trait bundles all four behind a single
//! associated type on [`Hook`](crate::Hook). Daemon provides filesystem
//! implementations; tests use in-memory backends.

pub mod agents;
#[cfg(feature = "test-utils")]
pub mod mem;
pub mod memory;
pub mod sessions;
pub mod skills;

pub use agents::AgentRepo;
pub use memory::{MemoryEntry, MemoryRepo, slugify};
pub use sessions::{SessionHandle, SessionRepo, SessionSnapshot, SessionSummary};
pub use skills::{Skill, SkillRepo};

use std::sync::Arc;

/// Composite persistence backend. [`Hook`](crate::Hook) exposes this as
/// a single associated type so `Env` stays at two generic parameters
/// (`H: Host`, `R: Repos`).
pub trait Repos: Send + Sync + 'static {
    type Memory: MemoryRepo;
    type Skills: SkillRepo;
    type Sessions: SessionRepo;
    type Agents: AgentRepo;

    fn memory(&self) -> &Arc<Self::Memory>;
    fn skills(&self) -> &Arc<Self::Skills>;
    fn sessions(&self) -> &Arc<Self::Sessions>;
    fn agents(&self) -> &Arc<Self::Agents>;
}

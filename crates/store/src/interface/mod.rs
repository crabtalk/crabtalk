//! What the runtime programs against.
//!
//! Five interfaces, every one of them fully written here. A trait is
//! bounded on the primitives it needs and its methods have bodies, so a
//! backend implements [`KVStorage`](crate::KVStorage) and
//! [`TextSearch`](crate::TextSearch) and *is* an `Agents`, a `Sessions`,
//! a `Memory` — there is nothing to construct and nothing to wire.
//!
//! That is the whole design. Nine methods stand between a new store and
//! a working daemon, and everything above them is written once.

//!
//! Nothing here returns a collection of bodies. Every listing is
//! identities or summaries, and the body is a second call for the one
//! thing the caller kept.

pub use agent::{Agents, validate_table_name};
pub use harness::Harnesses;
pub use memory::{Memory, MemoryEntry};
pub use session::{Sessions, Weights};
pub use skill::{Skill, SkillSummary, Skills};

mod agent;
mod harness;
mod memory;
mod session;
mod skill;

/// Everything the runtime needs from a store.
///
/// Blanket-implemented, like the five it bundles: it exists so
/// `Config::Storage` names one bound instead of five.
pub trait Backend: Agents + Sessions + Memory + Skills + Harnesses + Send + Sync + 'static {}

impl<T> Backend for T where
    T: Agents + Sessions + Memory + Skills + Harnesses + Send + Sync + 'static
{
}

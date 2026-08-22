//! What the runtime programs against.
//!
//! Six interfaces, each declared as what a store must answer and
//! nothing more. Every one is blanket-implemented over
//! [`KVStorage`](crate::KVStorage), so a backend that supplies the five
//! primitives *is* an `Agents`, a `Sessions`, a `Memory` — there is
//! nothing to construct and nothing to wire. A backend whose engine
//! already models these implements the interfaces directly instead, and
//! names no key.
//!
//! Nothing here returns a collection of bodies. Every listing is
//! identities or summaries, and the body is a second call for the one
//! thing the caller kept.

pub use agent::{Agents, validate_table_name};
pub use event::{EventSubscription, Subscriptions};
pub use harness::Harnesses;
pub use memory::{Memory, MemoryEntry};
pub use session::{Sessions, Weights};
pub use skill::{Skill, SkillSummary, Skills};

mod agent;
mod event;
mod harness;
mod memory;
mod session;
mod skill;

/// Everything the runtime needs from a store.
///
/// Blanket-implemented, like the six it bundles: it exists so
/// `Config::Storage` names one bound instead of six.
pub trait Backend: Agents + Sessions + Memory + Skills + Harnesses + Subscriptions {}

impl<T> Backend for T where T: Agents + Sessions + Memory + Skills + Harnesses + Subscriptions {}

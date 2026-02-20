//! Tower-inspired layer abstraction for agents.
//!
//! A [`Layer`] transforms one [`Agent`] into another, adding behavior
//! at compile time with zero dynamic dispatch overhead.
//!
//! # Example
//!
//! ```rust,ignore
//! use cydonia_core::Layer;
//!
//! let agent = MemoryLayer(memory)
//!     .layer(TeamLayer(analyst)
//!         .layer(PerpAgent::new(pool, &req)));
//! ```

use crate::Agent;

/// A layer that transforms one Agent into another.
///
/// Modeled after `tower::Layer` — each layer wraps an agent to add
/// behavior, producing a new agent type at compile time. Layers
/// compose by nesting, and the resulting type is fully monomorphized.
pub trait Layer<A: Agent> {
    /// The wrapped agent type produced by this layer.
    type Agent: Agent;

    /// Wrap the inner agent, producing a new agent with added behavior.
    fn layer(self, agent: A) -> Self::Agent;
}

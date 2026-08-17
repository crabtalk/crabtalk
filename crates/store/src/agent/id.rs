//! Stable agent identity.
//!
//! An [`AgentId`] is a ULID — Crockford base32, 26 characters, sortable
//! by creation time. Agents get a fresh ULID when they're created and
//! keep it across renames, which is why everything above storage keys on
//! it and a name is only ever a label.

use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    str::FromStr,
};
use ulid::Ulid;

/// Stable identifier for an agent. Newtype over [`Ulid`] so callers can
/// extend the type later without touching call sites.
///
/// The zero ULID is not a sentinel — it is the identity of the agent
/// seeded by [`Storage::scaffold`](crate::Storage::scaffold), so
/// [`Default`] names a real agent rather than a missing one. Absence is
/// spelled `Option<AgentId>`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct AgentId(pub Ulid);

impl AgentId {
    /// Generate a fresh ULID for a new agent.
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for AgentId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_str(s).map(Self)
    }
}

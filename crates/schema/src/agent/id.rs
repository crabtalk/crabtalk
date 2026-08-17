//! Stable agent identity.
//!
//! An [`AgentId`] is a ULID — Crockford base32, 26 characters, sortable
//! by creation time. Agents get a fresh ULID when they're created and
//! keep it across renames. Phase 5 introduces the field; later phases
//! (agent CRUD via Storage, sessions) start keying on it.

use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    str::FromStr,
};
use ulid::Ulid;

/// Stable identifier for an agent. Newtype over [`Ulid`] so callers can
/// extend the type later without touching call sites.
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

    /// The nil/zero ID — used as a sentinel for "not yet backfilled".
    /// Callers that see this on a registered agent should treat it as a
    /// bug: the daemon's startup backfill is expected to replace any
    /// missing `id` field before agents reach the runtime.
    pub const fn nil() -> Self {
        Self(Ulid::nil())
    }

    /// Is this the nil/zero sentinel?
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
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

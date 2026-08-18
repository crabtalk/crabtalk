use crate::SharedSession;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, AtomicUsize},
};
use store::AgentId;
use tokio_util::sync::CancellationToken;

/// What a client addresses a session by, and what the registry is keyed
/// on. The id is the wire's routing token, not an identity.
pub type Identity = (AgentId, String);

/// Both indexes, under one lock so they cannot disagree.
///
/// Sessions are addressed two ways and neither is secondary: clients
/// name an `(agent, sender)` pair, and tool replies come back carrying
/// the id. Scanning for either was the reason a lookup used to cost the
/// whole map.
#[derive(Default)]
pub struct Registry {
    pub by_id: BTreeMap<u64, Live>,
    pub by_identity: BTreeMap<Identity, u64>,
}

impl Registry {
    /// Register a session, displacing any live one with the same
    /// identity. Two live sessions for one pair would leave a lookup
    /// picking between them by id, so resuming an older session while
    /// one is open replaces it rather than shadowing it — the displaced
    /// one finishes any run in flight and persists, it just stops being
    /// addressable.
    pub fn insert(&mut self, id: u64, live: Live) {
        if let Some(displaced) = self.by_identity.insert(live.identity.clone(), id) {
            self.by_id.remove(&displaced);
        }
        self.by_id.insert(id, live);
    }

    pub fn remove(&mut self, id: u64) -> Option<Live> {
        let live = self.by_id.remove(&id)?;
        self.by_identity.remove(&live.identity);
        Some(live)
    }
}

/// One live session.
///
/// `identity` is copied out of the session so a lookup is readable
/// without taking its lock — that lock is held for a whole agent run,
/// so waiting on it would block behind an in-flight LLM call. Both
/// halves are immutable, so the copy cannot drift.
pub struct Live {
    pub identity: Identity,
    pub session: SharedSession,
    /// Cancellation token for the in-flight stream, present only while
    /// one is running. Kept beside the session rather than in a map of
    /// its own so closing one cannot leave the other behind.
    pub cancel: Option<CancellationToken>,
    /// Tick of the most recent lookup, for choosing what to evict.
    pub touched: AtomicU64,
    /// History size as of the last time this was readable. A session
    /// only grows during a run and a run holds the lock, so a stale
    /// figure is a floor rather than a guess.
    pub bytes: AtomicUsize,
}

use crate::SharedSession;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, AtomicUsize},
};
use store::SessionHandle;
use tokio_util::sync::CancellationToken;

/// Both indexes, under one lock so they cannot disagree.
///
/// Sessions are addressed two ways and neither is secondary: clients
/// name a handle, and tool replies come back carrying the id. Scanning
/// for either was the reason a lookup used to cost the whole map.
#[derive(Default)]
pub struct Registry {
    pub by_id: BTreeMap<u64, Live>,
    pub by_handle: BTreeMap<SessionHandle, u64>,
}

impl Registry {
    /// Register a session, displacing any live one under the same
    /// handle. A handle names one persisted session, so two live entries
    /// for it are a resolve racing a resolve — the second one replaces
    /// the first rather than shadowing it; the displaced one finishes
    /// any run in flight and persists, it just stops being addressable.
    pub fn insert(&mut self, id: u64, live: Live) {
        if let Some(displaced) = self.by_handle.insert(live.handle.clone(), id) {
            self.by_id.remove(&displaced);
        }
        self.by_id.insert(id, live);
    }

    pub fn remove(&mut self, id: u64) -> Option<Live> {
        let live = self.by_id.remove(&id)?;
        self.by_handle.remove(&live.handle);
        Some(live)
    }
}

/// One live session.
///
/// `handle` is copied out of the session so a lookup is readable
/// without taking its lock — that lock is held for a whole agent run,
/// so waiting on it would block behind an in-flight LLM call. Both
/// halves are immutable, so the copy cannot drift.
pub struct Live {
    pub handle: SessionHandle,
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

//! Live sessions — the registry, and the three fields it replaces.
//!
//! `id -> session`, `id -> steering sender` and the id counter were
//! three [`Runtime`] fields keyed by the same id, agreeing only by
//! convention: a close that missed the steering map leaked a sender, a
//! steer for a dead id sent into nothing. They are one structure here.
//!
//! It is not a `Runtime` field itself because a session outlives
//! the runtime that serves it — `reload` throws the `Runtime` away and
//! builds a new one, and nobody talking to the daemon should notice. The
//! runtime is handed a session to run against; which ones are live,
//! under what id, and who may steer them is this.
//!
//! What lives here is what the store cannot hold: a steering channel, a
//! lock that serializes runs against one session, and the id the wire
//! routes tool replies by. The history hanging off each one is a cache
//! of store content, which is why [`Sessions::max_live`] can drop it.

use crate::{
    Config, Runtime, Session, SharedSession,
    session::{Live, Registry},
};
use anyhow::Result;
use proto::ActiveConversationInfo;
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use store::{AgentId, interface::Sessions as _};
use tokio::sync::{Mutex, watch};

/// The daemon's live sessions.
///
/// The map is guarded by a synchronous lock and never held across an
/// await, so a stream's teardown can clean up from `Drop`.
pub struct Sessions {
    registry: parking_lot::RwLock<Registry>,
    next_id: AtomicU64,
    /// Monotonic access counter. Recency is a tick rather than a clock
    /// so a lookup can record one under the read lock.
    tick: AtomicU64,
    /// Bytes of history that may stay resident, or `None` for no bound.
    max_bytes: Option<usize>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Sessions {
    pub fn new(max_bytes: Option<usize>) -> Self {
        Self {
            registry: parking_lot::RwLock::new(Registry::default()),
            next_id: AtomicU64::new(1),
            tick: AtomicU64::new(0),
            max_bytes,
        }
    }

    /// The live session for an (agent, created_by) identity,
    /// resuming that identity's latest persisted session on first touch
    /// and starting an empty one if it has none.
    pub async fn get_or_create<C: Config>(
        &self,
        rt: &Runtime<C>,
        agent: &AgentId,
        created_by: &str,
    ) -> Result<(u64, SharedSession)> {
        if let Some(found) = self.find(agent, created_by) {
            // A session grows during its runs, and this is the only
            // moment one is both known and idle. Re-pricing it here is
            // what keeps the bound honest between inserts — otherwise a
            // stable set of sessions could grow past it forever, since
            // nothing new would ever arrive to trigger a sweep.
            self.reprice(found.0);
            return Ok(found);
        }
        if rt.agent(agent).await.is_none() {
            anyhow::bail!("agent '{agent}' not registered");
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let session = match rt.storage().find_latest_session(agent, created_by).await? {
            Some(handle) => rt.load(handle, id).await?,
            None => Session::new(id, agent, created_by),
        };
        Ok(self.insert(session))
    }

    /// Look up a live session by the identity clients address it by.
    pub fn find(&self, agent: &AgentId, created_by: &str) -> Option<(u64, SharedSession)> {
        let registry = self.registry.read();
        let id = *registry.by_identity.get(&(*agent, created_by.to_owned()))?;
        let live = registry.by_id.get(&id)?;
        self.touch(live);
        Some((id, live.session.clone()))
    }

    pub fn get(&self, id: u64) -> Option<SharedSession> {
        let registry = self.registry.read();
        let live = registry.by_id.get(&id)?;
        self.touch(live);
        Some(live.session.clone())
    }

    /// Re-measure one session and enforce the bound. Cheap: a walk of
    /// one history, then a sum of cached figures. The full sweep in
    /// [`evict`](Self::evict) only runs when that sum is over.
    fn reprice(&self, id: u64) {
        {
            let registry = self.registry.read();
            let Some(live) = registry.by_id.get(&id) else {
                return;
            };
            let Ok(session) = live.session.try_lock() else {
                return;
            };
            live.bytes.store(session.bytes(), Ordering::Relaxed);
            if self
                .max_bytes
                .is_none_or(|max| Self::resident(&registry) <= max)
            {
                return;
            }
        }
        self.evict(&mut self.registry.write());
    }

    fn touch(&self, live: &Live) {
        live.touched
            .store(self.tick.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
    }

    fn insert(&self, session: Session) -> (u64, SharedSession) {
        let id = session.id;
        let bytes = session.bytes();
        let live = Live {
            identity: (session.agent, session.created_by.clone()),
            session: Arc::new(Mutex::new(session)),
            steer: None,
            touched: AtomicU64::new(self.tick.fetch_add(1, Ordering::Relaxed)),
            bytes: AtomicUsize::new(bytes),
        };
        let shared = live.session.clone();
        let mut registry = self.registry.write();
        registry.insert(id, live);
        self.evict(&mut registry);
        (id, shared)
    }

    /// Drop least-recently-used sessions until resident history is
    /// within the bound.
    ///
    /// Only sessions nothing else holds are candidates. A runner owns an
    /// `Arc` for the length of its run, and evicting one would let the
    /// next message build a second live session for the same identity,
    /// reading a history the run has not finished writing.
    ///
    /// What goes is a cache entry rather than state: a run persists
    /// before it returns, so an evicted session reloads on next touch.
    fn evict(&self, registry: &mut Registry) {
        let Some(max) = self.max_bytes else { return };
        // Refresh what can be read without waiting. A locked session is
        // mid-run and keeps its last figure until that run ends.
        for live in registry.by_id.values() {
            if let Ok(session) = live.session.try_lock() {
                live.bytes.store(session.bytes(), Ordering::Relaxed);
            }
        }
        while Self::resident(registry) > max {
            let victim = registry
                .by_id
                .iter()
                .filter(|(_, live)| live.steer.is_none() && Arc::strong_count(&live.session) == 1)
                .min_by_key(|(_, live)| live.touched.load(Ordering::Relaxed))
                .map(|(id, _)| *id);
            // Everything left is in flight. The bound gives way rather
            // than the run.
            let Some(victim) = victim else { break };
            registry.remove(victim);
        }
    }

    /// Drop a session from the registry. Its steering channel goes
    /// with it, so a steer that arrives afterwards fails rather than
    /// disappearing into a sender nobody reads.
    pub fn close(&self, id: u64) -> bool {
        self.registry.write().remove(id).is_some()
    }

    /// Open this session's steering channel and hand back the
    /// receiving half for the stream about to run.
    pub fn begin_stream(&self, id: u64) -> Option<watch::Receiver<Option<String>>> {
        let (tx, rx) = watch::channel(None);
        let mut registry = self.registry.write();
        registry.by_id.get_mut(&id)?.steer = Some(tx);
        Some(rx)
    }

    /// Close the steering channel. Safe to call for an id that is already
    /// gone — a killed session takes its channel with it.
    pub fn end_stream(&self, id: u64) {
        if let Some(live) = self.registry.write().by_id.get_mut(&id) {
            live.steer = None;
        }
    }

    pub fn steer(&self, id: u64, content: String) -> Result<()> {
        let registry = self.registry.read();
        let tx = registry
            .by_id
            .get(&id)
            .and_then(|l| l.steer.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no active stream for session {id}"))?;
        tx.send(Some(content))
            .map_err(|_| anyhow::anyhow!("steering channel closed"))?;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.registry.read().by_id.len()
    }

    /// Bytes of history the registry is currently holding.
    pub fn resident_bytes(&self) -> usize {
        Self::resident(&self.registry.read())
    }

    fn resident(registry: &Registry) -> usize {
        registry
            .by_id
            .values()
            .map(|live| live.bytes.load(Ordering::Relaxed))
            .sum()
    }

    /// The live sessions, as the wire describes them. `rt` is here to
    /// put a name beside each id — the listing is read by a person.
    pub async fn list_active<C: Config>(&self, rt: &Runtime<C>) -> Vec<ActiveConversationInfo> {
        let entries: Vec<_> = {
            let registry = self.registry.read();
            registry
                .by_id
                .values()
                .map(|l| (l.identity.clone(), l.session.clone()))
                .collect()
        };
        let names = rt.agent_names().await;
        let mut infos = Vec::with_capacity(entries.len());
        for ((agent, sender), session) in entries {
            let c = session.lock().await;
            infos.push(ActiveConversationInfo {
                agent_id: agent.to_string(),
                agent_name: names.get(&agent).cloned().unwrap_or_default(),
                sender,
                message_count: c.history.len() as u64,
                alive_secs: c.created_at.elapsed().as_secs(),
                title: c.title.clone(),
            });
        }
        infos
    }
}

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

use crate::{Config, Runtime, Session, SharedSession};
use anyhow::Result;
use proto::ActiveConversationInfo;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use storage::{AgentId, Storage};
use tokio::sync::{Mutex, watch};

/// One live session.
///
/// `agent` and `created_by` are copied out of the session so its
/// identity is readable without taking its lock — that lock is held for
/// a whole agent run, so a lookup that waited on it would block behind
/// an in-flight LLM call. Both are immutable, so the copy cannot drift.
struct Live {
    agent: AgentId,
    created_by: String,
    session: SharedSession,
    /// Sender half of the steering channel, present only while a stream
    /// is running. Kept beside the session rather than in a map of
    /// its own so closing one cannot leave the other behind.
    steer: Option<watch::Sender<Option<String>>>,
}

/// The daemon's live sessions, keyed by runtime id.
///
/// The map is guarded by a synchronous lock and never held across an
/// await, so a stream's teardown can clean up from `Drop`.
pub struct Sessions {
    live: parking_lot::RwLock<BTreeMap<u64, Live>>,
    next_id: AtomicU64,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            live: parking_lot::RwLock::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl Sessions {
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
            return Ok(found);
        }
        if rt.agent(agent).is_none() {
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
        let live = self.live.read();
        live.iter()
            .find(|(_, l)| l.agent == *agent && l.created_by == created_by)
            .map(|(id, l)| (*id, l.session.clone()))
    }

    pub fn get(&self, id: u64) -> Option<SharedSession> {
        self.live.read().get(&id).map(|l| l.session.clone())
    }

    /// Register a session, displacing any live one with the same
    /// identity. Two live sessions for one (agent, created_by) pair
    /// would leave [`find`](Self::find) picking between them by id, so
    /// resuming an older session while one is open replaces it rather
    /// than shadowing it — the displaced one finishes any run in flight
    /// and persists, it just stops being addressable.
    fn insert(&self, session: Session) -> (u64, SharedSession) {
        let id = session.id;
        let live = Live {
            agent: session.agent,
            created_by: session.created_by.clone(),
            session: Arc::new(Mutex::new(session)),
            steer: None,
        };
        let shared = live.session.clone();
        let mut map = self.live.write();
        let displaced: Vec<u64> = map
            .iter()
            .filter(|(_, l)| l.agent == live.agent && l.created_by == live.created_by)
            .map(|(id, _)| *id)
            .collect();
        for id in displaced {
            map.remove(&id);
        }
        map.insert(id, live);
        (id, shared)
    }

    /// Drop a session from the registry. Its steering channel goes
    /// with it, so a steer that arrives afterwards fails rather than
    /// disappearing into a sender nobody reads.
    pub fn close(&self, id: u64) -> bool {
        self.live.write().remove(&id).is_some()
    }

    /// Open this session's steering channel and hand back the
    /// receiving half for the stream about to run.
    pub fn begin_stream(&self, id: u64) -> Option<watch::Receiver<Option<String>>> {
        let (tx, rx) = watch::channel(None);
        let mut live = self.live.write();
        live.get_mut(&id)?.steer = Some(tx);
        Some(rx)
    }

    /// Close the steering channel. Safe to call for an id that is already
    /// gone — a killed session takes its channel with it.
    pub fn end_stream(&self, id: u64) {
        if let Some(live) = self.live.write().get_mut(&id) {
            live.steer = None;
        }
    }

    pub fn steer(&self, id: u64, content: String) -> Result<()> {
        let live = self.live.read();
        let tx = live
            .get(&id)
            .and_then(|l| l.steer.as_ref())
            .ok_or_else(|| anyhow::anyhow!("no active stream for session {id}"))?;
        tx.send(Some(content))
            .map_err(|_| anyhow::anyhow!("steering channel closed"))?;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.live.read().len()
    }

    /// The live sessions, as the wire describes them. `rt` is here to
    /// put a name beside each id — the listing is read by a person.
    pub async fn list_active<C: Config>(&self, rt: &Runtime<C>) -> Vec<ActiveConversationInfo> {
        let entries: Vec<_> = {
            let live = self.live.read();
            live.values()
                .map(|l| (l.agent, l.created_by.clone(), l.session.clone()))
                .collect()
        };
        let mut infos = Vec::with_capacity(entries.len());
        for (agent, sender, session) in entries {
            let c = session.lock().await;
            infos.push(ActiveConversationInfo {
                agent_id: agent.to_string(),
                agent_name: rt.agent(&agent).map(|a| a.name).unwrap_or_default(),
                sender,
                message_count: c.history.len() as u64,
                alive_secs: c.created_at.elapsed().as_secs(),
                title: c.title.clone(),
            });
        }
        infos
    }
}

//! Gateway session management.
//!
//! Tracks active sessions with scoped trust levels and lifecycle
//! management. The gateway manages its own sessions externally from
//! the runtime's internal session tracking.

use compact_str::CompactString;
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Session scope determines isolation and tool access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionScope {
    /// Full session (WebSocket client, interactive).
    Main,
    /// Per-peer direct message session.
    Dm(CompactString),
    /// Per-group session.
    Group(CompactString),
    /// Per-cron-job session (fresh each run).
    Cron(CompactString),
}

/// Trust level for a session, determines tool access restrictions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    /// Unknown or unauthenticated sender.
    Untrusted,
    /// Authenticated user with restricted access.
    Trusted,
    /// Full administrative access.
    Admin,
}

/// An active gateway session.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier (UUID v4).
    pub id: CompactString,
    /// Session scope.
    pub scope: SessionScope,
    /// Trust level.
    pub trust_level: TrustLevel,
    /// Creation timestamp (unix seconds).
    pub created_at: u64,
    /// Last activity timestamp (unix seconds).
    pub last_active: u64,
}

/// Manages gateway sessions with thread-safe interior mutability.
pub struct SessionManager {
    sessions: Mutex<BTreeMap<CompactString, Session>>,
}

impl SessionManager {
    /// Create a new empty session manager.
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    /// Create a new session with the given scope and trust level.
    ///
    /// Returns the created session (cloned).
    pub fn create(&self, scope: SessionScope, trust_level: TrustLevel) -> Session {
        let now = unix_now();
        let id = CompactString::new(uuid::Uuid::new_v4().to_string());
        let session = Session {
            id: id.clone(),
            scope,
            trust_level,
            created_at: now,
            last_active: now,
        };
        self.sessions.lock().unwrap().insert(id, session.clone());
        session
    }

    /// Get a session by ID (cloned).
    pub fn get(&self, id: &str) -> Option<Session> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    /// Remove a session by ID.
    pub fn remove(&self, id: &str) -> Option<Session> {
        self.sessions.lock().unwrap().remove(id)
    }

    /// Update the last_active timestamp for a session.
    pub fn touch(&self, id: &str) {
        if let Some(session) = self.sessions.lock().unwrap().get_mut(id) {
            session.last_active = unix_now();
        }
    }

    /// Remove all sessions older than `max_age_secs` since last activity.
    pub fn cleanup_expired(&self, max_age_secs: u64) -> usize {
        let cutoff = unix_now().saturating_sub(max_age_secs);
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, s| s.last_active >= cutoff);
        before - sessions.len()
    }

    /// Get the number of active sessions.
    pub fn len(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Check if there are no active sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.lock().unwrap().is_empty()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

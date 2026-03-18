//! Lightweight task set — tracks delegated agent work as JoinHandles.
//!
//! Replaces the old TaskRegistry. A task is either running or done.
//! No status state machine, no queuing, no approval inbox.

use compact_str::CompactString;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use wcore::protocol::message::TaskInfo;

pub(crate) mod tool;

/// A delegated unit of agent work.
pub struct Task {
    /// Unique task identifier.
    pub id: u64,
    /// Agent assigned to this task.
    pub agent: CompactString,
    /// Human-readable task description / message.
    pub description: String,
    /// When this task was created.
    pub created_at: Instant,
    /// Session allocated for this task's execution.
    pub session_id: Option<u64>,
    /// Background handle — resolves to the agent's final response.
    pub handle: Option<JoinHandle<String>>,
    /// Cached result after handle resolves.
    pub result: Option<String>,
    /// Cached error after handle resolves.
    pub error: Option<String>,
}

impl Task {
    /// Whether the task has completed (handle resolved or result cached).
    pub fn is_done(&self) -> bool {
        self.result.is_some()
            || self.error.is_some()
            || self.handle.as_ref().is_some_and(|h| h.is_finished())
    }

    /// Status string for protocol compatibility.
    pub fn status(&self) -> &'static str {
        if self.error.is_some() {
            "failed"
        } else if self.result.is_some() || self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            "finished"
        } else {
            "in_progress"
        }
    }

    /// Build a `TaskInfo` snapshot for the wire protocol.
    pub fn to_info(&self) -> TaskInfo {
        TaskInfo {
            id: self.id,
            parent_id: None,
            agent: self.agent.to_string(),
            status: self.status().to_string(),
            description: self.description.clone(),
            result: self.result.clone(),
            error: self.error.clone(),
            created_by: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
            alive_secs: self.created_at.elapsed().as_secs(),
            blocked_on: None,
        }
    }
}

/// Lightweight task tracker — just a map of JoinHandles with metadata.
#[derive(Default)]
pub struct TaskSet {
    tasks: HashMap<u64, Task>,
    next_id: AtomicU64,
}

impl TaskSet {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Insert a new task. Returns the task ID.
    pub fn insert(&mut self, agent: CompactString, description: String) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.tasks.insert(
            id,
            Task {
                id,
                agent,
                description,
                created_at: Instant::now(),
                session_id: None,
                handle: None,
                result: None,
                error: None,
            },
        );
        id
    }

    /// Get a reference to a task.
    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Get a mutable reference to a task.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.get_mut(&id)
    }

    /// List all tasks (most recent first), up to `limit`.
    pub fn list(&self, limit: usize) -> Vec<&Task> {
        let mut tasks: Vec<_> = self.tasks.values().collect();
        tasks.sort_by(|a, b| b.id.cmp(&a.id));
        tasks.truncate(limit);
        tasks
    }

    /// Remove a task by ID, aborting its handle if running.
    pub fn kill(&mut self, id: u64) -> bool {
        if let Some(task) = self.tasks.remove(&id) {
            if let Some(handle) = task.handle {
                handle.abort();
            }
            true
        } else {
            false
        }
    }
}

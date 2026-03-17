//! Task registry — concurrency control and lifecycle.
//!
//! Pure data structure: no dispatch or spawning. Callers (hook, event loop)
//! own task execution; the registry just tracks state and broadcasts events.

use crate::hook::system::task::{InboxItem, Task, TaskStatus};
use compact_str::CompactString;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::time::Instant;
use wcore::protocol::message::{
    TaskCompleted, TaskCreated, TaskEvent, TaskInfo, TaskStatusChanged, task_event,
};

/// In-memory task registry with concurrency control.
pub struct TaskRegistry {
    tasks: BTreeMap<u64, Task>,
    next_id: AtomicU64,
    /// Maximum number of concurrently InProgress tasks.
    pub max_concurrent: usize,
    /// Maximum number of tasks returned by `list()`.
    pub viewable_window: usize,
    /// Broadcast channel for task lifecycle events (subscriptions).
    task_broadcast: broadcast::Sender<TaskEvent>,
}

impl TaskRegistry {
    /// Create a new registry with the given config.
    pub fn new(max_concurrent: usize, viewable_window: usize) -> Self {
        let (task_broadcast, _) = broadcast::channel(64);
        Self {
            tasks: BTreeMap::new(),
            next_id: AtomicU64::new(1),
            max_concurrent,
            viewable_window,
            task_broadcast,
        }
    }

    /// Subscribe to task lifecycle events.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.task_broadcast.subscribe()
    }

    /// Build a `TaskInfo` snapshot from an internal `Task`.
    pub fn task_info(task: &Task) -> TaskInfo {
        TaskInfo {
            id: task.id,
            parent_id: task.parent_id,
            agent: task.agent.to_string(),
            status: task.status.to_string(),
            description: task.description.clone(),
            result: task.result.clone(),
            error: task.error.clone(),
            created_by: task.created_by.to_string(),
            prompt_tokens: task.prompt_tokens,
            completion_tokens: task.completion_tokens,
            alive_secs: task.created_at.elapsed().as_secs(),
            blocked_on: task.blocked_on.as_ref().map(|i| i.question.clone()),
        }
    }

    /// Create a new task and insert it into the registry. Returns task ID.
    pub fn create(
        &mut self,
        agent: CompactString,
        description: String,
        created_by: CompactString,
        parent_id: Option<u64>,
        status: TaskStatus,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (status_tx, _) = watch::channel(status);
        let task = Task {
            id,
            parent_id,
            session_id: None,
            agent,
            status,
            created_by,
            description,
            result: None,
            error: None,
            blocked_on: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            created_at: Instant::now(),
            abort_handle: None,
            status_tx,
        };
        self.tasks.insert(id, task);
        if let Some(t) = self.tasks.get(&id) {
            let _ = self.task_broadcast.send(TaskEvent {
                event: Some(task_event::Event::Created(TaskCreated {
                    task: Some(Self::task_info(t)),
                })),
            });
        }
        id
    }

    /// Get a reference to a task by ID.
    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Get a mutable reference to a task by ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.get_mut(&id)
    }

    /// Update task status and notify all watchers (watch + broadcast).
    ///
    /// This is the **single path** for all status transitions.
    pub fn set_status(&mut self, id: u64, status: TaskStatus) {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.status = status;
            let _ = task.status_tx.send(status);
            let _ = self.task_broadcast.send(TaskEvent {
                event: Some(task_event::Event::StatusChanged(TaskStatusChanged {
                    task_id: id,
                    status: status.to_string(),
                    blocked_on: task.blocked_on.as_ref().map(|i| i.question.clone()),
                })),
            });
        }
    }

    /// Remove a task from the registry.
    pub fn remove(&mut self, id: u64) -> Option<Task> {
        self.tasks.remove(&id)
    }

    /// List tasks, most recent first, up to `viewable_window` entries.
    pub fn list(
        &self,
        agent: Option<&str>,
        status: Option<TaskStatus>,
        parent_id: Option<Option<u64>>,
    ) -> Vec<&Task> {
        self.tasks
            .values()
            .rev()
            .filter(|t| agent.is_none_or(|a| t.agent == a))
            .filter(|t| status.is_none_or(|s| t.status == s))
            .filter(|t| parent_id.is_none_or(|p| t.parent_id == p))
            .take(self.viewable_window)
            .collect()
    }

    /// Count of currently InProgress tasks (not Blocked).
    pub fn active_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::InProgress)
            .count()
    }

    /// Whether a new task can be dispatched immediately.
    pub fn has_slot(&self) -> bool {
        self.active_count() < self.max_concurrent
    }

    /// Mark a task as Finished or Failed and broadcast a Completed event.
    pub fn complete(&mut self, task_id: u64, result: Option<String>, error: Option<String>) {
        let status = if error.is_some() {
            TaskStatus::Failed
        } else {
            TaskStatus::Finished
        };
        if let Some(task) = self.tasks.get_mut(&task_id) {
            task.result = result.clone();
            task.error = error.clone();
        }
        self.set_status(task_id, status);
        let _ = self.task_broadcast.send(TaskEvent {
            event: Some(task_event::Event::Completed(TaskCompleted {
                task_id,
                status: status.to_string(),
                result,
                error,
            })),
        });
    }

    /// Find the next queued task and return its dispatch info, or `None`.
    pub fn promote_next(&mut self) -> Option<(u64, CompactString, String)> {
        if !self.has_slot() {
            return None;
        }
        let next = self
            .tasks
            .values()
            .find(|t| t.status == TaskStatus::Queued)
            .map(|t| (t.id, t.agent.clone(), t.description.clone()));
        if let Some((id, _, _)) = &next {
            self.set_status(*id, TaskStatus::InProgress);
        }
        next
    }

    /// Block a task for user approval. Returns a receiver for the response.
    pub fn block(&mut self, task_id: u64, question: String) -> Option<oneshot::Receiver<String>> {
        let task = self.tasks.get_mut(&task_id)?;
        let (tx, rx) = oneshot::channel();
        task.blocked_on = Some(InboxItem {
            question,
            reply: tx,
        });
        self.set_status(task_id, TaskStatus::Blocked);
        Some(rx)
    }

    /// Approve a blocked task, sending the response and resuming execution.
    pub fn approve(&mut self, task_id: u64, response: String) -> bool {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return false;
        };
        if task.status != TaskStatus::Blocked {
            return false;
        }
        if let Some(inbox) = task.blocked_on.take() {
            let _ = inbox.reply.send(response);
        }
        self.set_status(task_id, TaskStatus::InProgress);
        true
    }

    /// Kill a running or blocked task. Returns abort handle if it had one.
    pub fn kill(&mut self, task_id: u64) -> Option<tokio::task::AbortHandle> {
        let task = self.tasks.get_mut(&task_id)?;
        let handle = task.abort_handle.take();
        if let Some(ref h) = handle {
            h.abort();
        }
        task.error = Some("killed by user".into());
        self.set_status(task_id, TaskStatus::Failed);
        handle
    }

    /// Subscribe to a task's status changes (for await_tasks).
    pub fn subscribe_status(&self, task_id: u64) -> Option<watch::Receiver<TaskStatus>> {
        self.tasks.get(&task_id).map(|t| t.status_tx.subscribe())
    }

    /// Get all child tasks of a given parent.
    pub fn children(&self, parent_id: u64) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| t.parent_id == Some(parent_id))
            .collect()
    }

    /// Find a task by its session ID. Returns the task ID.
    pub fn find_by_session(&self, session_id: u64) -> Option<u64> {
        self.tasks
            .values()
            .find(|t| t.session_id == Some(session_id))
            .map(|t| t.id)
    }

    /// Add token usage to a task.
    pub fn add_tokens(&mut self, task_id: u64, prompt: u64, completion: u64) {
        if let Some(task) = self.tasks.get_mut(&task_id) {
            task.prompt_tokens += prompt;
            task.completion_tokens += completion;
        }
    }
}

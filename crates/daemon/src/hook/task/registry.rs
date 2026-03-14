//! Task registry — concurrency control, dispatch, and lifecycle.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use crate::hook::task::{InboxItem, Task, TaskStatus};
use compact_str::CompactString;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot, watch};
use tokio::time::Instant;
use wcore::protocol::message::{
    ClientMessage, KillMsg, SendMsg, ServerMessage, TaskCompleted, TaskCreated, TaskEvent,
    TaskInfo, TaskStatusChanged, client_message, server_message, task_event,
};

/// In-memory task registry with concurrency control.
pub struct TaskRegistry {
    tasks: BTreeMap<u64, Task>,
    next_id: AtomicU64,
    /// Maximum number of concurrently InProgress tasks.
    pub max_concurrent: usize,
    /// Maximum number of tasks returned by `list()`.
    pub viewable_window: usize,
    /// Per-task execution timeout.
    pub task_timeout: Duration,
    /// Event channel for dispatching task execution.
    pub event_tx: DaemonEventSender,
    /// Broadcast channel for task lifecycle events (subscriptions).
    task_broadcast: broadcast::Sender<TaskEvent>,
}

impl TaskRegistry {
    /// Create a new registry with the given config and event sender.
    pub fn new(
        max_concurrent: usize,
        viewable_window: usize,
        task_timeout: Duration,
        event_tx: DaemonEventSender,
    ) -> Self {
        let (task_broadcast, _) = broadcast::channel(64);
        Self {
            tasks: BTreeMap::new(),
            next_id: AtomicU64::new(1),
            max_concurrent,
            viewable_window,
            task_timeout,
            event_tx,
            task_broadcast,
        }
    }

    /// Subscribe to task lifecycle events.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.task_broadcast.subscribe()
    }

    /// Build a `TaskInfo` snapshot from an internal `Task`.
    fn task_info(task: &Task) -> TaskInfo {
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

    /// Create a new task and insert it into the registry.
    pub fn create(
        &mut self,
        agent: CompactString,
        description: String,
        created_by: CompactString,
        parent_id: Option<u64>,
        status: TaskStatus,
        spawned: bool,
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
            spawned,
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

    /// Update task status and notify watchers.
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
    ///
    /// Optionally filters by agent, status, or parent_id.
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

    /// Submit a task for execution.
    ///
    /// If under the concurrency limit, dispatches immediately and spawns a
    /// watcher. Otherwise, queues the task. Returns `(task_id, status)`.
    pub fn submit(
        &mut self,
        agent: CompactString,
        message: String,
        created_by: CompactString,
        parent_id: Option<u64>,
        registry: Arc<Mutex<TaskRegistry>>,
    ) -> (u64, TaskStatus) {
        let under_limit = self.active_count() < self.max_concurrent;
        let initial_status = if under_limit {
            TaskStatus::InProgress
        } else {
            TaskStatus::Queued
        };

        let task_id = self.create(
            agent.clone(),
            message.clone(),
            created_by,
            parent_id,
            initial_status,
            true,
        );

        if under_limit {
            self.dispatch_task(task_id, agent, message, registry);
        }

        (task_id, initial_status)
    }

    /// Dispatch a task: send the message via event channel and spawn a watcher.
    fn dispatch_task(
        &mut self,
        task_id: u64,
        agent: CompactString,
        message: String,
        registry: Arc<Mutex<TaskRegistry>>,
    ) {
        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        let msg = ClientMessage::from(SendMsg {
            agent: agent.to_string(),
            content: message,
            session: None,
            sender: None,
        });
        let _ = self.event_tx.send(DaemonEvent::Message {
            msg,
            reply: reply_tx,
        });

        let event_tx = self.event_tx.clone();
        let timeout = self.task_timeout;
        let handle = tokio::spawn(task_watcher(task_id, reply_rx, registry, event_tx, timeout));
        if let Some(task) = self.tasks.get_mut(&task_id) {
            task.abort_handle = Some(handle.abort_handle());
        }
    }

    /// Mark a task as Finished or Failed, then promote the next queued task.
    pub fn complete(
        &mut self,
        task_id: u64,
        result: Option<String>,
        error: Option<String>,
        registry: Arc<Mutex<TaskRegistry>>,
    ) {
        if let Some(task) = self.tasks.get_mut(&task_id) {
            if error.is_some() {
                task.status = TaskStatus::Failed;
                task.error = error.clone();
                let _ = task.status_tx.send(TaskStatus::Failed);
                let _ = self.task_broadcast.send(TaskEvent {
                    event: Some(task_event::Event::Completed(TaskCompleted {
                        task_id,
                        status: TaskStatus::Failed.to_string(),
                        result: None,
                        error,
                    })),
                });
            } else {
                task.status = TaskStatus::Finished;
                task.result = result.clone();
                let _ = task.status_tx.send(TaskStatus::Finished);
                let _ = self.task_broadcast.send(TaskEvent {
                    event: Some(task_event::Event::Completed(TaskCompleted {
                        task_id,
                        status: TaskStatus::Finished.to_string(),
                        result,
                        error: None,
                    })),
                });
            }
        }
        self.promote_next(registry);
    }

    /// Promote the next queued task to InProgress if a slot is available.
    pub fn promote_next(&mut self, registry: Arc<Mutex<TaskRegistry>>) {
        if self.active_count() >= self.max_concurrent {
            return;
        }
        // Find the oldest queued task.
        let next = self
            .tasks
            .values()
            .find(|t| t.status == TaskStatus::Queued)
            .map(|t| (t.id, t.agent.clone(), t.description.clone()));

        if let Some((id, agent, message)) = next {
            self.set_status(id, TaskStatus::InProgress);
            self.dispatch_task(id, agent, message, registry);
        }
    }

    /// Block a task, setting status to Blocked and storing the inbox item.
    ///
    /// Returns a receiver that the tool call can await for the user's response.
    pub fn block(&mut self, task_id: u64, question: String) -> Option<oneshot::Receiver<String>> {
        let task = self.tasks.get_mut(&task_id)?;
        let (tx, rx) = oneshot::channel();
        task.blocked_on = Some(InboxItem {
            question,
            reply: tx,
        });
        task.status = TaskStatus::Blocked;
        let _ = task.status_tx.send(TaskStatus::Blocked);
        let _ = self.task_broadcast.send(TaskEvent {
            event: Some(task_event::Event::StatusChanged(TaskStatusChanged {
                task_id,
                status: TaskStatus::Blocked.to_string(),
                blocked_on: task.blocked_on.as_ref().map(|i| i.question.clone()),
            })),
        });
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
        task.status = TaskStatus::InProgress;
        let _ = task.status_tx.send(TaskStatus::InProgress);
        let _ = self.task_broadcast.send(TaskEvent {
            event: Some(task_event::Event::StatusChanged(TaskStatusChanged {
                task_id,
                status: TaskStatus::InProgress.to_string(),
                blocked_on: None,
            })),
        });
        true
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

    /// Collect queued `create_task` entries grouped by agent.
    ///
    /// Returns `(agent, [(task_id, description)])` pairs, capped at
    /// `max_concurrent` tasks per agent to avoid context overflow.
    pub fn queued_create_tasks(&self) -> BTreeMap<CompactString, Vec<(u64, String)>> {
        let mut groups: BTreeMap<CompactString, Vec<(u64, String)>> = BTreeMap::new();
        for task in self.tasks.values() {
            if task.status == TaskStatus::Queued && !task.spawned {
                let entry = groups.entry(task.agent.clone()).or_default();
                if entry.len() < self.max_concurrent {
                    entry.push((task.id, task.description.clone()));
                }
            }
        }
        groups
    }

    /// Collect queued `create_task` entries for a single agent, capped at
    /// `max_concurrent`.
    pub fn queued_create_tasks_for(&self, agent: &str) -> Vec<(u64, String)> {
        let mut entries = Vec::new();
        for task in self.tasks.values() {
            if task.status == TaskStatus::Queued && !task.spawned && task.agent == agent {
                entries.push((task.id, task.description.clone()));
                if entries.len() >= self.max_concurrent {
                    break;
                }
            }
        }
        entries
    }
}

/// Watcher task: awaits reply messages with timeout, closes session, completes task.
async fn task_watcher(
    task_id: u64,
    mut reply_rx: mpsc::UnboundedReceiver<ServerMessage>,
    registry: Arc<Mutex<TaskRegistry>>,
    event_tx: DaemonEventSender,
    timeout: Duration,
) {
    let mut result_content: Option<String> = None;
    let mut error_msg: Option<String> = None;
    let mut session_id: Option<u64> = None;

    let collect = async {
        while let Some(msg) = reply_rx.recv().await {
            match msg.msg {
                Some(server_message::Msg::Response(resp)) => {
                    session_id = Some(resp.session);
                    result_content = Some(resp.content);
                }
                Some(server_message::Msg::Error(err)) => {
                    error_msg = Some(err.message);
                }
                _ => {}
            }
        }
    };

    if tokio::time::timeout(timeout, collect).await.is_err() {
        error_msg = Some("task timed out".into());
    }

    // Close the session to prevent accumulation.
    if let Some(sid) = session_id {
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let _ = event_tx.send(DaemonEvent::Message {
            msg: ClientMessage {
                msg: Some(client_message::Msg::Kill(KillMsg { session: sid })),
            },
            reply: reply_tx,
        });
    }

    // Complete the task, auto-close sub-task sessions, and promote next queued.
    let reg = registry.clone();
    let mut locked = registry.lock().await;
    // Collect finished sub-task session IDs for auto-close.
    let child_sessions: Vec<u64> = locked
        .children(task_id)
        .iter()
        .filter(|t| t.status == TaskStatus::Finished || t.status == TaskStatus::Failed)
        .filter_map(|t| t.session_id)
        .collect();
    locked.complete(task_id, result_content, error_msg, reg);
    drop(locked);

    // Auto-close finished sub-task sessions outside the lock.
    for sid in child_sessions {
        let (reply_tx, _) = mpsc::unbounded_channel();
        let _ = event_tx.send(DaemonEvent::Message {
            msg: ClientMessage {
                msg: Some(client_message::Msg::Kill(KillMsg { session: sid })),
            },
            reply: reply_tx,
        });
    }
}

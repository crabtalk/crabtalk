//! Tool dispatch, schema registration, and task watcher for task tools.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use crate::hook::system::task::TaskStatus;
use crate::hook::{DaemonHook, system::task::TaskRegistry};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
    protocol::message::{
        ClientMessage, KillMsg, SendMsg, ServerMessage, client_message, server_message,
    },
};

// ── Dispatch helpers on DaemonHook ──────────────────────────────────

impl DaemonHook {
    pub(crate) async fn dispatch_spawn_task(
        &self,
        args: &str,
        agent: &str,
        parent_task_id: Option<u64>,
    ) -> String {
        let input: SpawnTask = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        // Enforce members scope.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.members.is_empty()
            && !scope.members.iter().any(|m| m == &input.agent)
        {
            return format!("agent '{}' is not in your members list", input.agent);
        }
        let registry = self.tasks.clone();
        let mut reg = registry.lock().await;
        let under_limit = reg.has_slot();
        let initial_status = if under_limit {
            TaskStatus::InProgress
        } else {
            TaskStatus::Queued
        };
        let task_id = reg.create(
            input.agent.into(),
            input.message.clone(),
            agent.into(),
            parent_task_id,
            initial_status,
        );
        if under_limit {
            let agent_name = reg.get(task_id).unwrap().agent.clone();
            dispatch_task(
                task_id,
                agent_name,
                input.message,
                registry.clone(),
                self.event_tx.clone(),
                self.task_timeout,
                &mut reg,
            );
        }
        drop(reg);
        serde_json::json!({ "task_id": task_id, "status": initial_status.to_string() }).to_string()
    }

    pub(crate) async fn dispatch_check_tasks(&self, args: &str) -> String {
        let input: CheckTasks = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let status_filter = input.status.as_deref().and_then(parse_task_status);
        let registry = self.tasks.lock().await;
        let tasks = registry.list(
            input.agent.as_deref(),
            status_filter,
            input.parent_id.map(Some),
        );
        let entries: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "task_id": t.id,
                    "agent": t.agent.as_str(),
                    "status": t.status.to_string(),
                    "description": t.description,
                    "parent_id": t.parent_id,
                    "result": t.result,
                    "error": t.error,
                    "created_by": t.created_by.as_str(),
                    "alive_secs": t.created_at.elapsed().as_secs(),
                    "prompt_tokens": t.prompt_tokens,
                    "completion_tokens": t.completion_tokens,
                })
            })
            .collect();
        serde_json::to_string(&entries).unwrap_or_else(|e| format!("serialization error: {e}"))
    }

    pub(crate) async fn dispatch_ask_user(&self, args: &str, task_id: Option<u64>) -> String {
        let input: AskUser = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let Some(tid) = task_id else {
            return "ask_user can only be called from within a task context".to_owned();
        };
        let rx = {
            let mut registry = self.tasks.lock().await;
            match registry.block(tid, input.question) {
                Some(rx) => rx,
                None => return format!("task {tid} not found"),
            }
        };
        match rx.await {
            Ok(response) => response,
            Err(_) => "user did not respond (channel closed)".to_owned(),
        }
    }

    pub(crate) async fn dispatch_await_tasks(&self, args: &str, task_id: Option<u64>) -> String {
        let input: AwaitTasks = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.task_ids.is_empty() {
            return "no task IDs provided".to_owned();
        }
        // Subscribe to status changes and optionally block ourselves.
        let mut receivers = Vec::new();
        {
            let mut registry = self.tasks.lock().await;
            for &tid in &input.task_ids {
                match registry.subscribe_status(tid) {
                    Some(rx) => receivers.push((tid, rx)),
                    None => return format!("task {tid} not found"),
                }
            }
            if let Some(tid) = task_id {
                registry.set_status(tid, TaskStatus::Blocked);
            }
        }
        // Wait for all tasks to reach Finished or Failed.
        for (_, rx) in &mut receivers {
            let mut rx = rx.clone();
            loop {
                let status = *rx.borrow_and_update();
                if status == TaskStatus::Finished || status == TaskStatus::Failed {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
        // Unblock ourselves and collect results.
        if let Some(tid) = task_id {
            self.tasks
                .lock()
                .await
                .set_status(tid, TaskStatus::InProgress);
        }
        let registry = self.tasks.lock().await;
        let results: Vec<serde_json::Value> = input
            .task_ids
            .iter()
            .map(|&tid| {
                if let Some(t) = registry.get(tid) {
                    serde_json::json!({
                        "task_id": tid,
                        "status": t.status.to_string(),
                        "result": t.result,
                        "error": t.error,
                    })
                } else {
                    serde_json::json!({ "task_id": tid, "status": "not_found" })
                }
            })
            .collect();
        serde_json::to_string(&results).unwrap_or_else(|e| format!("serialization error: {e}"))
    }
}

// ── Task dispatch and watcher (free functions) ──────────────────────

/// Dispatch a task: send the message via event channel and spawn a watcher.
/// Must be called while holding the registry lock (to set abort_handle).
pub(crate) fn dispatch_task(
    task_id: u64,
    agent: compact_str::CompactString,
    message: String,
    registry: Arc<Mutex<TaskRegistry>>,
    event_tx: DaemonEventSender,
    timeout: Duration,
    reg: &mut TaskRegistry,
) {
    let (reply_tx, reply_rx) = mpsc::unbounded_channel();
    let msg = ClientMessage::from(SendMsg {
        agent: agent.to_string(),
        content: message,
        session: None,
        sender: None,
    });
    let _ = event_tx.send(DaemonEvent::Message {
        msg,
        reply: reply_tx,
    });

    let handle = tokio::spawn(task_watcher(task_id, reply_rx, registry, event_tx, timeout));
    if let Some(task) = reg.get_mut(task_id) {
        task.abort_handle = Some(handle.abort_handle());
    }
}

/// Promote the next queued task if a slot opened up.
pub(crate) fn try_promote(
    reg: &mut TaskRegistry,
    registry: Arc<Mutex<TaskRegistry>>,
    event_tx: DaemonEventSender,
    timeout: Duration,
) {
    if let Some((id, agent, message)) = reg.promote_next() {
        dispatch_task(id, agent, message, registry, event_tx, timeout, reg);
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

    // Close the task's own session.
    if let Some(sid) = session_id {
        send_kill(&event_tx, sid);
    }

    // Complete task, collect child sessions, promote next.
    let mut reg = registry.lock().await;
    let child_sessions: Vec<u64> = reg
        .children(task_id)
        .iter()
        .filter(|t| t.status == TaskStatus::Finished || t.status == TaskStatus::Failed)
        .filter_map(|t| t.session_id)
        .collect();
    reg.complete(task_id, result_content, error_msg);
    try_promote(&mut reg, registry.clone(), event_tx.clone(), timeout);
    drop(reg);

    // Auto-close finished sub-task sessions outside the lock.
    for sid in child_sessions {
        send_kill(&event_tx, sid);
    }
}

/// Send a kill message for a session.
fn send_kill(event_tx: &DaemonEventSender, session: u64) {
    let (reply_tx, _) = mpsc::unbounded_channel();
    let _ = event_tx.send(DaemonEvent::Message {
        msg: ClientMessage {
            msg: Some(client_message::Msg::Kill(KillMsg { session })),
        },
        reply: reply_tx,
    });
}

// ── Status parsing ──────────────────────────────────────────────────

fn parse_task_status(s: &str) -> Option<TaskStatus> {
    match s {
        "queued" => Some(TaskStatus::Queued),
        "in_progress" => Some(TaskStatus::InProgress),
        "blocked" => Some(TaskStatus::Blocked),
        "finished" => Some(TaskStatus::Finished),
        "failed" => Some(TaskStatus::Failed),
        _ => None,
    }
}

// ── Tool schemas ────────────────────────────────────────────────────

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        SpawnTask::as_tool(),
        CheckTasks::as_tool(),
        AskUser::as_tool(),
        AwaitTasks::as_tool(),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct SpawnTask {
    /// Target agent name to delegate the task to.
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
}

impl ToolDescription for SpawnTask {
    const DESCRIPTION: &'static str = "Delegate an async task to another agent. Returns task_id and status (in_progress or queued). Use check_tasks to monitor progress.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct CheckTasks {
    /// Filter by agent name.
    #[serde(default)]
    pub agent: Option<String>,
    /// Filter by status (queued, in_progress, blocked, finished, failed).
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by parent task ID.
    #[serde(default)]
    pub parent_id: Option<u64>,
}

impl ToolDescription for CheckTasks {
    const DESCRIPTION: &'static str = "Query the task registry. Filterable by agent, status, parent_id. Returns up to 16 most recent tasks.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct AskUser {
    /// Question to ask the user.
    pub question: String,
}

impl ToolDescription for AskUser {
    const DESCRIPTION: &'static str = "Ask the user a question. Blocks the current task until the user responds. Only works within a task context.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct AwaitTasks {
    /// Task IDs to wait for.
    pub task_ids: Vec<u64>,
}

impl ToolDescription for AwaitTasks {
    const DESCRIPTION: &'static str =
        "Block until the specified tasks finish. Returns collected results for each task.";
}

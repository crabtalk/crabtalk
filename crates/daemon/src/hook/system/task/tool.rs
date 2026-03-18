//! Tool schemas and dispatch for delegation tools.
//!
//! Three tools: delegate, collect, check_tasks.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use crate::hook::DaemonHook;
use crate::hook::system::task::TaskSet;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
    protocol::message::{ClientMessage, SendMsg, ServerMessage, server_message},
};

// ── Dispatch helpers on DaemonHook ──────────────────────────────────

impl DaemonHook {
    pub(crate) async fn dispatch_delegate(&self, args: &str, agent: &str) -> String {
        let input: Delegate = match serde_json::from_str(args) {
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

        let mut tasks = self.tasks.lock().await;
        let task_id = tasks.insert(input.agent.clone().into(), input.message.clone());

        // Spawn agent via event channel and collect result in background.
        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        let msg = ClientMessage::from(SendMsg {
            agent: input.agent,
            content: input.message,
            session: None,
            sender: None,
        });
        let _ = self.event_tx.send(DaemonEvent::Message {
            msg,
            reply: reply_tx,
        });

        let tasks_arc = self.tasks.clone();
        let event_tx = self.event_tx.clone();
        let handle = tokio::spawn(collect_result(task_id, reply_rx, tasks_arc, event_tx));
        if let Some(task) = tasks.get_mut(task_id) {
            task.handle = Some(handle);
        }
        drop(tasks);

        serde_json::json!({ "task_id": task_id }).to_string()
    }

    pub(crate) async fn dispatch_collect(&self, args: &str) -> String {
        let input: Collect = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.task_ids.is_empty() {
            return "no task IDs provided".to_owned();
        }

        // Wait for all specified tasks to complete.
        let mut handles = Vec::new();
        {
            let mut tasks = self.tasks.lock().await;
            for &tid in &input.task_ids {
                if let Some(task) = tasks.get_mut(tid)
                    && let Some(handle) = task.handle.take()
                {
                    handles.push((tid, handle));
                }
            }
        }

        // Await all handles outside the lock.
        for (tid, handle) in handles {
            let _ = handle.await;
            // Result is already stored by collect_result.
            // Just ensure we waited.
            let _ = tid;
        }

        // Collect results.
        let tasks = self.tasks.lock().await;
        let results: Vec<serde_json::Value> = input
            .task_ids
            .iter()
            .map(|&tid| {
                if let Some(t) = tasks.get(tid) {
                    serde_json::json!({
                        "task_id": tid,
                        "status": t.status(),
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

    pub(crate) async fn dispatch_check_tasks(&self, args: &str) -> String {
        let input: CheckTasks = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        let tasks = self.tasks.lock().await;
        let all = tasks.list(16);
        let filtered: Vec<_> = all
            .into_iter()
            .filter(|t| input.agent.as_deref().is_none_or(|a| t.agent == a))
            .collect();
        let entries: Vec<serde_json::Value> = filtered
            .iter()
            .map(|t| {
                serde_json::json!({
                    "task_id": t.id,
                    "agent": t.agent.as_str(),
                    "status": t.status(),
                    "description": t.description,
                    "result": t.result,
                    "error": t.error,
                    "alive_secs": t.created_at.elapsed().as_secs(),
                })
            })
            .collect();
        serde_json::to_string(&entries).unwrap_or_else(|e| format!("serialization error: {e}"))
    }
}

// ── Background result collector ─────────────────────────────────────

/// Collect the agent's response from the reply channel and store it on the task.
async fn collect_result(
    task_id: u64,
    mut reply_rx: mpsc::UnboundedReceiver<ServerMessage>,
    tasks: Arc<Mutex<TaskSet>>,
    event_tx: DaemonEventSender,
) -> String {
    let mut result_content: Option<String> = None;
    let mut error_msg: Option<String> = None;
    let mut session_id: Option<u64> = None;

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

    // Close the agent's session.
    if let Some(sid) = session_id {
        let (reply_tx, _) = mpsc::unbounded_channel();
        let _ = event_tx.send(DaemonEvent::Message {
            msg: ClientMessage {
                msg: Some(wcore::protocol::message::client_message::Msg::Kill(
                    wcore::protocol::message::KillMsg { session: sid },
                )),
            },
            reply: reply_tx,
        });
    }

    // Store result on the task.
    let result = result_content.clone().unwrap_or_default();
    let mut reg = tasks.lock().await;
    if let Some(task) = reg.get_mut(task_id) {
        task.session_id = session_id;
        task.result = result_content;
        task.error = error_msg;
    }
    result
}

// ── Tool schemas ────────────────────────────────────────────────────

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Delegate::as_tool(),
        Collect::as_tool(),
        CheckTasks::as_tool(),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Delegate {
    /// Target agent name to delegate the task to.
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
}

impl ToolDescription for Delegate {
    const DESCRIPTION: &'static str = "Delegate a task to another agent. The agent runs in an isolated context and returns a compact result. Use collect to gather results.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Collect {
    /// Task IDs to wait for and collect results from.
    pub task_ids: Vec<u64>,
}

impl ToolDescription for Collect {
    const DESCRIPTION: &'static str =
        "Wait for delegated tasks to complete and collect their results.";
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct CheckTasks {
    /// Filter by agent name.
    #[serde(default)]
    pub agent: Option<String>,
}

impl ToolDescription for CheckTasks {
    const DESCRIPTION: &'static str = "List delegated tasks with their current status.";
}

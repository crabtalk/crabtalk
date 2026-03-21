//! Tool schema and dispatch for the delegate tool.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use crate::hook::DaemonHook;
use serde::Deserialize;
use tokio::sync::mpsc;
use wcore::{
    agent::{AsTool, ToolDescription},
    model::Tool,
    protocol::message::{ClientMessage, SendMsg, server_message},
};

pub(crate) fn tools() -> Vec<Tool> {
    vec![Delegate::as_tool()]
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct Delegate {
    /// List of tasks to run in parallel. Each task has an agent name and a message.
    pub tasks: Vec<DelegateTask>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct DelegateTask {
    /// Target agent name.
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
}

impl ToolDescription for Delegate {
    const DESCRIPTION: &'static str = "Delegate tasks to other agents. Runs all tasks in parallel, blocks until all complete, and returns their results.";
}

impl DaemonHook {
    pub(crate) async fn dispatch_delegate(&self, args: &str, agent: &str) -> String {
        let input: Delegate = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.tasks.is_empty() {
            return "no tasks provided".to_owned();
        }

        // Enforce members scope for all target agents.
        if let Some(scope) = self.scopes.get(agent)
            && !scope.members.is_empty()
        {
            for task in &input.tasks {
                if !scope.members.iter().any(|m| m == &task.agent) {
                    return format!("agent '{}' is not in your members list", task.agent);
                }
            }
        }

        // Spawn all tasks in parallel.
        let mut handles = Vec::with_capacity(input.tasks.len());
        for task in input.tasks {
            let handle = spawn_agent_task(task.agent.clone(), task.message, self.event_tx.clone());
            handles.push((task.agent, handle));
        }

        // Wait for all and collect results.
        let mut results = Vec::with_capacity(handles.len());
        for (agent_name, handle) in handles {
            let (result, error) = match handle.await {
                Ok((r, e)) => (r, e),
                Err(e) => (None, Some(format!("task panicked: {e}"))),
            };
            results.push(serde_json::json!({
                "agent": agent_name,
                "result": result,
                "error": error,
            }));
        }

        serde_json::to_string(&results).unwrap_or_else(|e| format!("serialization error: {e}"))
    }
}

/// Spawn an agent task via the event channel and collect its response.
fn spawn_agent_task(
    agent: String,
    message: String,
    event_tx: DaemonEventSender,
) -> tokio::task::JoinHandle<(Option<String>, Option<String>)> {
    tokio::spawn(async move {
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel();
        let msg = ClientMessage::from(SendMsg {
            agent,
            content: message,
            session: None,
            sender: None,
            cwd: None,
        });
        if event_tx
            .send(DaemonEvent::Message {
                msg,
                reply: reply_tx,
            })
            .is_err()
        {
            return (None, Some("event channel closed".to_owned()));
        }

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

        (result_content, error_msg)
    })
}

//! Delegate tool handler factory.

use crate::node::event::{NodeEvent, NodeEventSender};
use runtime::AgentScope;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot};
use wcore::{
    ToolDispatch, ToolHandler,
    agent::{AsTool, ToolDescription},
    model::Tool,
    protocol::message::{ClientMessage, SendMsg, server_message},
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Delegate {
    /// List of tasks to run in parallel. Each task has an agent name and a message.
    pub tasks: Vec<DelegateTask>,
    /// If true, return immediately with task IDs instead of waiting for completion.
    /// Results arrive via agent completion events (`agent:{name}:done`).
    #[serde(default)]
    pub background: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DelegateTask {
    /// Target agent name. Auto-generated if empty and system_prompt is set.
    #[serde(default)]
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
    /// System prompt for an ephemeral agent. When set, creates a temporary
    /// agent with this prompt instead of targeting a pre-registered agent.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Working directory for this task. Defaults to the parent's CWD.
    #[serde(default)]
    pub cwd: Option<String>,
}

impl ToolDescription for Delegate {
    const DESCRIPTION: &'static str = "Delegate tasks to other agents. Runs all tasks in parallel. Set background=true to return immediately with task IDs — results arrive via agent completion events.";
}

pub fn handler(
    event_tx: NodeEventSender,
    scopes: Arc<RwLock<BTreeMap<String, AgentScope>>>,
) -> (Tool, ToolHandler) {
    (
        Delegate::as_tool(),
        Arc::new(move |call: ToolDispatch| {
            let event_tx = event_tx.clone();
            let scopes = scopes.clone();
            Box::pin(async move {
                let input: Delegate = serde_json::from_str(&call.args)
                    .map_err(|e| format!("invalid arguments: {e}"))?;
                if input.tasks.is_empty() {
                    return Err("no tasks provided".to_owned());
                }
                {
                    let scopes = scopes.read().expect("scopes lock poisoned");
                    if let Some(scope) = scopes.get(&call.agent)
                        && !scope.members.is_empty()
                    {
                        for task in &input.tasks {
                            if !scope.members.iter().any(|m| m == &task.agent) {
                                return Err(format!(
                                    "agent '{}' is not in your members list",
                                    task.agent
                                ));
                            }
                        }
                    }
                }
                dispatch_delegate(input, &event_tx).await
            })
        }),
    )
}

async fn dispatch_delegate(input: Delegate, event_tx: &NodeEventSender) -> Result<String, String> {
    let mut ephemeral_names = Vec::new();
    let mut tasks = Vec::with_capacity(input.tasks.len());
    for task in input.tasks {
        let agent_name = if let Some(prompt) = task.system_prompt {
            let name = if task.agent.is_empty() {
                ephemeral_agent_name()
            } else {
                task.agent
            };
            let mut config = wcore::AgentConfig::new(&name);
            config.system_prompt = prompt;
            let (tx, rx) = oneshot::channel();
            let _ = event_tx.send(NodeEvent::AddEphemeral { config, reply: tx });
            let _ = rx.await;
            ephemeral_names.push(name.clone());
            name
        } else {
            task.agent
        };

        let sender = delegate_sender();
        let handle = spawn_agent_task(
            agent_name.clone(),
            task.message,
            task.cwd,
            sender.clone(),
            event_tx.clone(),
        );
        tasks.push((agent_name, sender, handle));
    }

    if input.background {
        let mut json_results = Vec::with_capacity(tasks.len());
        let mut handles = Vec::with_capacity(tasks.len());
        for (agent, sender, handle) in tasks {
            json_results.push(serde_json::json!({ "agent": agent, "task_id": sender }));
            handles.push(handle);
        }
        if !ephemeral_names.is_empty() {
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                for h in handles {
                    let _ = h.await;
                }
                for name in ephemeral_names {
                    let _ = event_tx.send(NodeEvent::RemoveEphemeral { name });
                }
            });
        }
        return serde_json::to_string(&json_results)
            .map_err(|e| format!("serialization error: {e}"));
    }

    let mut results = Vec::with_capacity(tasks.len());
    for (agent_name, _sender, handle) in tasks {
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

    for name in ephemeral_names {
        let _ = event_tx.send(NodeEvent::RemoveEphemeral { name });
    }

    serde_json::to_string(&results).map_err(|e| format!("serialization error: {e}"))
}

fn delegate_sender() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("delegate:{id}")
}

fn ephemeral_agent_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("_ephemeral:{id}")
}

fn spawn_agent_task(
    agent: String,
    message: String,
    cwd: Option<String>,
    delegate_sender: String,
    event_tx: NodeEventSender,
) -> tokio::task::JoinHandle<(Option<String>, Option<String>)> {
    tokio::spawn(async move {
        let (reply_tx, mut reply_rx) = mpsc::channel(transport::REPLY_CHANNEL_CAPACITY);
        let msg = ClientMessage::from(SendMsg {
            agent: agent.clone(),
            content: message,
            sender: Some(delegate_sender.clone()),
            cwd,
            guest: None,
            tool_choice: None,
        });
        if event_tx
            .send(NodeEvent::Message {
                msg,
                reply: reply_tx,
            })
            .is_err()
        {
            return (None, Some("event channel closed".to_owned()));
        }

        let mut result_content: Option<String> = None;
        let mut error_msg: Option<String> = None;

        while let Some(msg) = reply_rx.recv().await {
            match msg.msg {
                Some(server_message::Msg::Response(resp)) => {
                    result_content = Some(resp.content);
                }
                Some(server_message::Msg::Error(err)) => {
                    error_msg = Some(err.message);
                }
                _ => {}
            }
        }

        let (reply_tx, _) = mpsc::channel(1);
        let _ = event_tx.send(NodeEvent::Message {
            msg: ClientMessage {
                msg: Some(wcore::protocol::message::client_message::Msg::Kill(
                    wcore::protocol::message::KillMsg {
                        agent,
                        sender: delegate_sender,
                    },
                )),
            },
            reply: reply_tx,
        });

        (result_content, error_msg)
    })
}

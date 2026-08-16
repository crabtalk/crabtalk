//! Delegate — agent orchestration, run from the client.
//!
//! The daemon knows nothing about this tool: the client declares it in
//! `StreamMsg.tools` and the daemon forwards the call back like any other
//! client tool. Fan-out is then just more streams on the same socket — a
//! conversation is keyed by `(agent, sender)`, so a distinct sender buys
//! each sub-agent its own conversation, its own bridge listener, and its
//! own tool set.

use crate::repl::tools::sub_agent_tools;
use client::{ConnectionInfo, StreamAccumulator};
use serde::Deserialize;
use tokio::sync::mpsc;
use wcore::protocol::message::{AgentInfo, StreamMsg, ToolCallForwardEvent, stream_event};

/// Delegate tasks to other agents. Runs all tasks in parallel and returns
/// their results once every task has finished.
///
/// Each task targets an agent by name — the agent must already exist.
/// Sub-agents run with the OS tools; they cannot question the user or
/// delegate further.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Delegate {
    /// Tasks to run in parallel.
    pub tasks: Vec<DelegateTask>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DelegateTask {
    /// Name of the agent to run this task.
    pub agent: String,
    /// Instruction for the target agent.
    pub message: String,
}

/// Name and describe the agents `delegate` can target, for the model to
/// choose from. Composed here rather than daemon-side because the client
/// decides whether it offers `delegate` at all — a stream that withholds
/// the tool should say nothing about targets it cannot reach.
///
/// Empty when there are no peers, so the caller can skip the block.
pub fn peers_block(agents: &[AgentInfo], self_name: &str) -> String {
    let peers: Vec<&AgentInfo> = agents
        .iter()
        .filter(|a| a.name != self_name && !a.description.is_empty())
        .collect();
    if peers.is_empty() {
        return String::new();
    }
    let mut block = String::from("<agents>\n");
    for peer in peers {
        block.push_str(&format!("- {}: {}\n", peer.name, peer.description));
    }
    block.push_str("</agents>\n\n");
    block
}

/// Fan-out state reported back to the REPL as it happens. Without this the
/// parent stream is silent for the whole delegation and the UI looks hung.
pub enum Progress {
    /// Tasks accepted, in the order their indices refer to.
    Started {
        call_id: String,
        agents: Vec<String>,
    },
    /// A running task's current tool calls, as `(name, arguments)` pairs.
    /// Empty means the step finished and the row should go quiet again.
    /// Left unformatted so the REPL can label them at its render width,
    /// in the same vocabulary as the main transcript.
    Active {
        call_id: String,
        index: usize,
        calls: Vec<(String, String)>,
    },
    /// One task settled.
    Finished {
        call_id: String,
        index: usize,
        ok: bool,
        detail: String,
    },
}

/// Run every task in parallel and return the JSON array handed back to the
/// model as the tool result.
pub async fn execute(
    conn: &ConnectionInfo,
    args: &str,
    call_id: &str,
    progress: mpsc::UnboundedSender<Progress>,
) -> Result<String, String> {
    let input: Delegate =
        serde_json::from_str(args).map_err(|e| format!("invalid arguments: {e}"))?;
    if input.tasks.is_empty() {
        return Err("no tasks provided".to_owned());
    }
    let _ = progress.send(Progress::Started {
        call_id: call_id.to_owned(),
        agents: input.tasks.iter().map(|t| t.agent.clone()).collect(),
    });

    let results =
        futures_util::future::join_all(input.tasks.into_iter().enumerate().map(|(i, task)| {
            run_task(
                conn.clone(),
                task,
                format!("delegate:{call_id}:{i}"),
                Reporter {
                    tx: progress.clone(),
                    call_id: call_id.to_owned(),
                    index: i,
                },
            )
        }))
        .await;
    serde_json::to_string(&results).map_err(|e| format!("serialization error: {e}"))
}

/// A single task's handle on the progress channel.
struct Reporter {
    tx: mpsc::UnboundedSender<Progress>,
    call_id: String,
    index: usize,
}

impl Reporter {
    fn active(&self, calls: Vec<(String, String)>) {
        let _ = self.tx.send(Progress::Active {
            call_id: self.call_id.clone(),
            index: self.index,
            calls,
        });
    }

    fn finish(self, ok: bool, detail: String) {
        let _ = self.tx.send(Progress::Finished {
            call_id: self.call_id,
            index: self.index,
            ok,
            detail,
        });
    }
}

async fn run_task(
    conn: ConnectionInfo,
    task: DelegateTask,
    sender: String,
    reporter: Reporter,
) -> serde_json::Value {
    let req = StreamMsg {
        agent: task.agent.clone(),
        content: task.message,
        sender: Some(sender.clone()),
        tools: sub_agent_tools(),
        ..Default::default()
    };

    let mut rx = conn.stream(req);
    let mut acc = StreamAccumulator::new();
    let mut transport_error = None;
    while let Some(event) = rx.recv().await {
        match event {
            Ok(event) => {
                match &event {
                    stream_event::Event::ToolCallForward(forward) => {
                        refuse(conn.clone(), forward.clone());
                    }
                    stream_event::Event::ToolStart(start) => reporter.active(
                        start
                            .calls
                            .iter()
                            .map(|c| (c.name.clone(), c.arguments.clone()))
                            .collect(),
                    ),
                    stream_event::Event::ToolsComplete(_) => reporter.active(Vec::new()),
                    _ => {}
                }
                acc.push(&event);
            }
            Err(e) => transport_error = Some(e.to_string()),
        }
    }
    let _ = conn.kill_conversation(task.agent.clone(), sender).await;

    let error = transport_error.or_else(|| acc.error().map(str::to_owned));
    reporter.finish(
        error.is_none(),
        error.clone().unwrap_or_else(|| headline(acc.text())),
    );

    serde_json::json!({
        "agent": task.agent,
        "result": acc.text(),
        "error": error,
    })
}

/// First non-empty line of a sub-agent's answer — enough to tell at a glance
/// what came back without dumping the whole reply into the task list.
fn headline(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Answer a forward a sub-agent should never have received.
///
/// `sub_agent_tools` declares nothing, so the daemon has nothing to forward —
/// but an unanswered forward is a hang until the timeout rather than a
/// fallback, so this replies with an error instead of dropping it.
fn refuse(conn: ConnectionInfo, forward: ToolCallForwardEvent) {
    tokio::spawn(async move {
        let _ = conn
            .reply_to_tool(
                forward.conversation_id,
                forward.call_id,
                format!("{}: sub-agents have no client tools", forward.name),
                true,
            )
            .await;
    });
}

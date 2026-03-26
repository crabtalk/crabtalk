//! DaemonBridge — server-specific RuntimeBridge implementation.
//!
//! Provides `ask_user` and `delegate` dispatch using daemon event channels,
//! and per-session CWD resolution.

use crate::daemon::event::{DaemonEvent, DaemonEventSender};
use runtime::bridge::RuntimeBridge;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc, oneshot};
use wcore::protocol::message::{ClientMessage, SendMsg, server_message};

/// Timeout for waiting on user reply (5 minutes).
const ASK_USER_TIMEOUT: Duration = Duration::from_secs(300);

/// Server-specific bridge for the daemon. Owns event channels and session state.
pub struct DaemonBridge {
    /// Event channel for task delegation.
    pub event_tx: DaemonEventSender,
    /// Pending `ask_user` oneshots, keyed by session_id.
    pub pending_asks: Arc<Mutex<HashMap<u64, oneshot::Sender<String>>>>,
    /// Per-session working directory overrides.
    pub session_cwds: Arc<Mutex<HashMap<u64, PathBuf>>>,
}

impl RuntimeBridge for DaemonBridge {
    async fn dispatch_ask_user(&self, args: &str, session_id: Option<u64>) -> String {
        let input: runtime::ask_user::AskUser = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };

        let session_id = match session_id {
            Some(id) => id,
            None => return "ask_user is only available in streaming mode".to_owned(),
        };

        let (tx, rx) = oneshot::channel();
        self.pending_asks.lock().await.insert(session_id, tx);

        match tokio::time::timeout(ASK_USER_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => {
                self.pending_asks.lock().await.remove(&session_id);
                "ask_user cancelled: reply channel closed".to_owned()
            }
            Err(_) => {
                self.pending_asks.lock().await.remove(&session_id);
                let headers: Vec<&str> =
                    input.questions.iter().map(|q| q.header.as_str()).collect();
                format!(
                    "ask_user timed out after {}s: no reply received for: {}",
                    ASK_USER_TIMEOUT.as_secs(),
                    headers.join("; "),
                )
            }
        }
    }

    async fn dispatch_delegate(&self, args: &str, _agent: &str) -> String {
        let input: runtime::task::Delegate = match serde_json::from_str(args) {
            Ok(v) => v,
            Err(e) => return format!("invalid arguments: {e}"),
        };
        if input.tasks.is_empty() {
            return "no tasks provided".to_owned();
        }

        // Note: member scope enforcement is handled by RuntimeHook::dispatch_tool
        // before delegating to the bridge. The bridge doesn't need access to scopes.

        let mut handles = Vec::with_capacity(input.tasks.len());
        for task in input.tasks {
            let handle = spawn_agent_task(task.agent.clone(), task.message, self.event_tx.clone());
            handles.push((task.agent, handle));
        }

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

    fn session_cwd(&self, session_id: u64) -> Option<PathBuf> {
        self.session_cwds
            .try_lock()
            .ok()
            .and_then(|m| m.get(&session_id).cloned())
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
            new_chat: false,
            resume_file: None,
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

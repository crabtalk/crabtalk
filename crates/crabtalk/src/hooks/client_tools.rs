//! Client-tools hook — forwards dispatches over the active stream and
//! awaits a `ReplyToTool` from the connected client.
//!
//! The daemon never executes the OS tool set itself. Instead it advertises
//! the schemas (sourced from [`sdk::tools::os::schemas`]) so the LLM sees
//! them, and when the agent dispatches one of those names this hook parks
//! a oneshot keyed by `tool_call.id`. The protocol layer emits a
//! `ToolCallForward` event on the same stream; the client (TUI today)
//! dispatches locally and posts a `ReplyToTool` on a fresh connection,
//! which resolves the oneshot via [`ClientToolHook::resolve`].

use runtime::Hook;
use std::{collections::HashMap, time::Duration};
use tokio::sync::{Mutex, oneshot};
use wcore::{ToolDispatch, ToolFuture, model::Tool};

/// How long a forwarded call waits for a `ReplyToTool` before failing.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(300);

/// Hook that forwards OS-tool dispatches to a connected client.
pub struct ClientToolHook {
    /// Schemas advertised to the LLM.
    schemas: Vec<Tool>,
    /// Names this hook claims. Membership check on every dispatch.
    names: Vec<String>,
    /// Conversations whose stream is currently listening for forwarded calls.
    listeners: Mutex<HashMap<u64, ()>>,
    /// Pending forwarded calls keyed by `tool_call.id`.
    pending: Mutex<HashMap<String, oneshot::Sender<Result<String, String>>>>,
}

impl ClientToolHook {
    pub fn new() -> Self {
        let schemas = sdk::tools::os::schemas();
        let names = sdk::tools::os::names();
        Self {
            schemas,
            names,
            listeners: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `name` is part of the client-tool set this hook forwards.
    /// Used by the protocol layer to decide when to emit a
    /// `ToolCallForward` event alongside the usual `ToolStart`.
    pub fn is_client_tool(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// Mark `conversation_id` as having an active stream that will handle
    /// forwarded calls. Without this the hook's dispatch fails fast
    /// instead of hanging until timeout.
    pub async fn register_listener(&self, conversation_id: u64) {
        self.listeners.lock().await.insert(conversation_id, ());
    }

    /// Drop the listener registration. Pending oneshots for this
    /// conversation are not eagerly cleaned up — they will fail when the
    /// client disconnects (sender dropped) or hit the timeout.
    pub async fn unregister_listener(&self, conversation_id: u64) {
        self.listeners.lock().await.remove(&conversation_id);
    }

    /// Resolve a pending forwarded call. Returns `true` if a pending
    /// oneshot was found and dispatched. Used by the `ReplyToTool` server
    /// handler.
    pub async fn try_resolve(&self, call_id: &str, output: String, is_error: bool) -> bool {
        let Some(tx) = self.pending.lock().await.remove(call_id) else {
            return false;
        };
        let payload = if is_error { Err(output) } else { Ok(output) };
        let _ = tx.send(payload);
        true
    }
}

impl Default for ClientToolHook {
    fn default() -> Self {
        Self::new()
    }
}

impl Hook for ClientToolHook {
    fn schema(&self) -> Vec<Tool> {
        self.schemas.clone()
    }

    fn system_prompt(&self) -> Option<String> {
        let mut buf = String::from("\n\n<environment>\n");
        buf.push_str(&format!("os: {}\n", std::env::consts::OS));
        buf.push_str("</environment>");
        Some(buf)
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if !self.is_client_tool(name) {
            return None;
        }
        Some(Box::pin(async move {
            let Some(conv_id) = call.conversation_id else {
                return Err(format!("'{name}' requires a conversation context"));
            };
            if !self.listeners.lock().await.contains_key(&conv_id) {
                return Err(format!(
                    "no client connected to handle '{name}' for this conversation"
                ));
            }
            if call.call_id.is_empty() {
                return Err(format!("'{name}' invoked without a call_id"));
            }

            let (tx, rx) = oneshot::channel();
            self.pending.lock().await.insert(call.call_id.clone(), tx);

            match tokio::time::timeout(FORWARD_TIMEOUT, rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => {
                    self.pending.lock().await.remove(&call.call_id);
                    Err(format!("'{name}' cancelled: reply channel closed"))
                }
                Err(_) => {
                    self.pending.lock().await.remove(&call.call_id);
                    Err(format!(
                        "'{name}' timed out after {}s",
                        FORWARD_TIMEOUT.as_secs()
                    ))
                }
            }
        }))
    }
}

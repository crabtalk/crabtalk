//! Client bridge — forwards tool dispatches to the connected client and
//! awaits replies.
//!
//! This is the client-side dispatch layer. What the daemon serves itself
//! dispatches through the composite harness. Client tools
//! dispatch through this bridge: the protocol layer emits a
//! `ToolCallForward` event, the client executes locally, and posts a reply
//! which resolves via [`ClientBridge::try_resolve`].
//!
//! A client's tools are exactly what it declares in `StreamMsg.tools`.
//! There is no default set: the daemon cannot execute a client tool, so
//! advertising one the client never claimed only buys a forward nobody
//! answers — a hang until the timeout, not a fallback. Sets are kept per
//! conversation, so clients with different capabilities can be connected
//! at once.
//!
//! Which tools those are is not this crate's business. It forwards the
//! names it was handed and never inspects them.

use parking_lot::Mutex;
use schema::{ToolDispatch, ToolFuture, model::Tool};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::sync::oneshot;

/// How long a forwarded call waits for a reply before failing.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(300);

enum PendingState {
    AwaitingReply(oneshot::Sender<Result<String, String>>),
    EarlyReply(Result<String, String>),
}

type PendingKey = (u64, String);

/// Bridge that forwards client-tool dispatches over the active stream.
#[derive(Default)]
pub struct ClientBridge {
    /// Declared tool names per conversation. Only names are kept — the
    /// schemas go to the model, not through here.
    conversations: Mutex<HashMap<u64, HashSet<String>>>,
    listeners: Mutex<HashSet<u64>>,
    pending: Mutex<HashMap<PendingKey, PendingState>>,
}

impl ClientBridge {
    /// Record what the client declared for this conversation.
    pub fn register_tools(&self, conversation_id: u64, tools: &[Tool]) {
        let names = tools.iter().map(|t| t.function.name.clone()).collect();
        self.conversations.lock().insert(conversation_id, names);
    }

    /// Whether `name` is a client tool for this conversation. False when
    /// nothing was declared — there is nowhere to forward it.
    pub fn is_client_tool(&self, conversation_id: u64, name: &str) -> bool {
        self.conversations
            .lock()
            .get(&conversation_id)
            .is_some_and(|names| names.contains(name))
    }

    /// Mark `conversation_id` as having an active stream listener.
    pub fn register_listener(&self, conversation_id: u64) {
        self.listeners.lock().insert(conversation_id);
    }

    /// Drop the listener and clean up per-conversation state.
    pub fn unregister_listener(&self, conversation_id: u64) {
        self.listeners.lock().remove(&conversation_id);
        self.conversations.lock().remove(&conversation_id);
        let mut pending = self.pending.lock();
        let keys: Vec<PendingKey> = pending
            .keys()
            .filter(|(c, _)| *c == conversation_id)
            .cloned()
            .collect();
        for key in keys {
            if let Some(PendingState::AwaitingReply(tx)) = pending.remove(&key) {
                let _ = tx.send(Err("stream closed before reply arrived".to_owned()));
            }
        }
    }

    /// Resolve a forwarded call. Returns `false` on duplicate reply.
    pub fn try_resolve(
        &self,
        conversation_id: u64,
        call_id: &str,
        output: String,
        is_error: bool,
    ) -> bool {
        let result = if is_error { Err(output) } else { Ok(output) };
        let key = (conversation_id, call_id.to_owned());
        let mut pending = self.pending.lock();
        match pending.remove(&key) {
            Some(PendingState::AwaitingReply(tx)) => {
                let _ = tx.send(result);
                true
            }
            Some(PendingState::EarlyReply(_)) => false,
            None => {
                pending.insert(key, PendingState::EarlyReply(result));
                true
            }
        }
    }

    /// Dispatch a client tool call. Returns `None` if this bridge doesn't
    /// own the tool for the given conversation.
    pub fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        let conv_id = call.conversation_id?;
        if !self.is_client_tool(conv_id, name) {
            return None;
        }
        Some(Box::pin(async move {
            if !self.listeners.lock().contains(&conv_id) {
                return Err(format!(
                    "no client connected to handle '{name}' for this conversation"
                ));
            }
            if call.call_id.is_empty() {
                return Err(format!("'{name}' invoked without a call_id"));
            }

            let key = (conv_id, call.call_id.clone());
            let rx = {
                let mut pending = self.pending.lock();
                match pending.remove(&key) {
                    Some(PendingState::EarlyReply(result)) => return result,
                    Some(PendingState::AwaitingReply(_)) => {
                        return Err(format!(
                            "'{name}' has a duplicate pending dispatch for call_id '{}'",
                            call.call_id
                        ));
                    }
                    None => {
                        let (tx, rx) = oneshot::channel();
                        pending.insert(key.clone(), PendingState::AwaitingReply(tx));
                        rx
                    }
                }
            };

            match tokio::time::timeout(FORWARD_TIMEOUT, rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => {
                    self.pending.lock().remove(&key);
                    Err(format!("'{name}' cancelled: reply channel closed"))
                }
                Err(_) => {
                    self.pending.lock().remove(&key);
                    Err(format!(
                        "'{name}' timed out after {}s",
                        FORWARD_TIMEOUT.as_secs()
                    ))
                }
            }
        }))
    }
}

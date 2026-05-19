//! Client bridge — forwards tool dispatches to the connected client and
//! awaits replies.
//!
//! This is the client-side dispatch layer. System capabilities (memory,
//! delegate, sessions, skill, mcp) dispatch through daemon-side hooks.
//! Client tools (bash, read, edit, ask_user) dispatch through this bridge:
//! the protocol layer emits a `ToolCallForward` event, the client executes
//! locally, and posts a reply which resolves via [`ClientBridge::try_resolve`].
//!
//! Pending calls are keyed by `(conversation_id, call_id)`. The map handles
//! the dispatch/reply race symmetrically: if the reply arrives before the
//! dispatch parks, it's stashed as `EarlyReply`.

use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::sync::oneshot;
use wcore::{ToolDispatch, ToolFuture, model::Tool};

/// How long a forwarded call waits for a reply before failing.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(300);

enum PendingState {
    AwaitingReply(oneshot::Sender<Result<String, String>>),
    EarlyReply(Result<String, String>),
}

type PendingKey = (u64, String);

/// Bridge that forwards client-tool dispatches over the active stream.
pub struct ClientBridge {
    schemas: Vec<Tool>,
    names: Vec<String>,
    listeners: Mutex<HashSet<u64>>,
    pending: Mutex<HashMap<PendingKey, PendingState>>,
}

impl ClientBridge {
    pub fn new() -> Self {
        let mut schemas = sdk::tools::os::schemas();
        schemas.push(sdk::tools::ask_user::schema());
        let mut names = sdk::tools::os::names();
        names.push(sdk::tools::ask_user::name());
        Self {
            schemas,
            names,
            listeners: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Tool schemas this bridge provides (for merging into ToolRegistry).
    pub fn schemas(&self) -> Vec<Tool> {
        self.schemas.clone()
    }

    /// Whether `name` is a client tool this bridge handles.
    pub fn is_client_tool(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// Mark `conversation_id` as having an active stream listener.
    pub fn register_listener(&self, conversation_id: u64) {
        self.listeners.lock().insert(conversation_id);
    }

    /// Drop the listener and fail-fast any pending calls for this
    /// conversation.
    pub fn unregister_listener(&self, conversation_id: u64) {
        self.listeners.lock().remove(&conversation_id);
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
    /// own the tool.
    pub fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if !self.is_client_tool(name) {
            return None;
        }
        Some(Box::pin(async move {
            let Some(conv_id) = call.conversation_id else {
                return Err(format!("'{name}' requires a conversation context"));
            };
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

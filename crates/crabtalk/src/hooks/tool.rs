//! Tool forwarding hook — forwards dispatches over the active stream and
//! awaits a `ReplyToTool` from the connected client.
//!
//! Tools that execute on the client side (OS tools, ask_user) are
//! advertised by this hook so the LLM sees them. When the agent dispatches
//! one, this hook hands the call off to the connected client: the protocol
//! layer emits a `ToolCallForward` event on the same stream, the client
//! dispatches locally, and posts `ReplyToTool` which resolves via
//! [`ToolHook::try_resolve`].
//!
//! Pending calls are keyed by `(conversation_id, call_id)`. The LLM's
//! `call_id` is not globally unique across conversations, so we namespace
//! it. The map also handles the dispatch/reply race symmetrically: if the
//! reply arrives before the agent's dispatch parks, we stash it as
//! `EarlyReply` and the dispatch picks it up immediately.

use parking_lot::Mutex;
use runtime::Hook;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::sync::oneshot;
use wcore::{ToolDispatch, ToolFuture, model::Tool};

/// How long a forwarded call waits for a `ReplyToTool` before failing.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(300);

/// State of a pending forwarded call.
enum PendingState {
    /// Agent's dispatch parked first; reply will resolve the sender.
    AwaitingReply(oneshot::Sender<Result<String, String>>),
    /// Reply arrived first; dispatch will take the stashed result.
    EarlyReply(Result<String, String>),
}

type PendingKey = (u64, String);

/// Hook that forwards OS-tool dispatches to a connected client.
pub struct ToolHook {
    /// Schemas advertised to the LLM.
    schemas: Vec<Tool>,
    /// Names this hook claims. Membership check on every dispatch.
    names: Vec<String>,
    /// Conversations whose stream is currently listening for forwarded calls.
    listeners: Mutex<HashSet<u64>>,
    /// Pending forwarded calls keyed by `(conversation_id, call_id)`.
    pending: Mutex<HashMap<PendingKey, PendingState>>,
}

impl ToolHook {
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

    /// Whether `name` is part of the client-tool set this hook forwards.
    pub fn is_client_tool(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// Mark `conversation_id` as having an active stream listener.
    /// Synchronous so RAII guards can call it from `Drop`.
    pub fn register_listener(&self, conversation_id: u64) {
        self.listeners.lock().insert(conversation_id);
    }

    /// Drop the listener and fail-fast any pending calls for this
    /// conversation. `AwaitingReply` entries get a stream-closed error;
    /// `EarlyReply` entries (unclaimed by a dispatch) are dropped.
    /// Synchronous so RAII guards can call it from `Drop`.
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

    /// Resolve a forwarded call. If the agent's dispatch has already parked,
    /// fires the oneshot. Otherwise stashes the result as `EarlyReply` so
    /// the upcoming dispatch picks it up. Returns `false` only on duplicate
    /// reply for the same call (rare, ignored).
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
}

impl Default for ToolHook {
    fn default() -> Self {
        Self::new()
    }
}

impl Hook for ToolHook {
    fn schema(&self) -> Vec<Tool> {
        self.schemas.clone()
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
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

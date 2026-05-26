//! Client bridge — forwards tool dispatches to the connected client and
//! awaits replies.
//!
//! This is the client-side dispatch layer. System capabilities (memory,
//! delegate, sessions, skill, mcp) dispatch through daemon-side hooks.
//! Client tools dispatch through this bridge: the protocol layer emits a
//! `ToolCallForward` event, the client executes locally, and posts a reply
//! which resolves via [`ClientBridge::try_resolve`].
//!
//! Clients declare their tools at stream/send time via the `tools` field
//! on `StreamMsg`/`SendMsg`. When the field is empty, built-in defaults
//! (bash, read, edit, ask_user) are used for backwards compatibility.
//! Per-conversation tool sets are stored so `dispatch` and `is_client_tool`
//! route correctly when different clients bring different tools.

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
    defaults: ClientToolSet,
    conversations: Mutex<HashMap<u64, ClientToolSet>>,
    listeners: Mutex<HashSet<u64>>,
    pending: Mutex<HashMap<PendingKey, PendingState>>,
}

struct ClientToolSet {
    schemas: Vec<Tool>,
    names: HashSet<String>,
}

impl ClientToolSet {
    fn new(schemas: Vec<Tool>) -> Self {
        let names = schemas.iter().map(|t| t.function.name.clone()).collect();
        Self { schemas, names }
    }
}

impl ClientBridge {
    pub fn new() -> Self {
        let mut schemas = hooks::os::schemas();
        schemas.push(sdk::tools::ask_user::schema());
        Self {
            defaults: ClientToolSet::new(schemas),
            conversations: Mutex::new(HashMap::new()),
            listeners: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Default tool schemas used when clients don't declare their own.
    pub fn default_schemas(&self) -> Vec<Tool> {
        self.defaults.schemas.clone()
    }

    /// Register client-provided tools for a conversation. These override
    /// the built-in defaults for dispatch and forwarding within this
    /// conversation.
    pub fn register_tools(&self, conversation_id: u64, tools: Vec<Tool>) {
        self.conversations
            .lock()
            .insert(conversation_id, ClientToolSet::new(tools));
    }

    /// Return the effective tool schemas for a conversation — per-
    /// conversation if registered, otherwise the built-in defaults.
    pub fn effective_tools(&self, conversation_id: u64) -> Vec<Tool> {
        let conversations = self.conversations.lock();
        conversations
            .get(&conversation_id)
            .unwrap_or(&self.defaults)
            .schemas
            .clone()
    }

    /// Whether `name` is a client tool for the given conversation.
    pub fn is_client_tool(&self, conversation_id: u64, name: &str) -> bool {
        let conversations = self.conversations.lock();
        conversations
            .get(&conversation_id)
            .unwrap_or(&self.defaults)
            .names
            .contains(name)
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

//! Env — trait for node-specific capabilities and tool dispatch.
//!
//! The runtime engine talks to a single `Env` implementation. The node
//! crate provides [`NodeEnv`] which bundles event broadcasting,
//! instruction discovery, and a composite Hook. Tests use `()`.

use crate::Hook;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use wcore::{AgentEvent, ToolDispatch, ToolFuture, protocol::message};

/// The runtime environment — combines server capabilities with tool dispatch.
///
/// Each node/binary defines one implementation that wires together
/// the composite hook, event broadcasting, CWD management, and
/// instruction discovery.
pub trait Env: Send + Sync + 'static {
    /// The composite hook providing tool schemas, dispatch, and lifecycle.
    type Hook: Hook;

    /// Access the composite hook.
    fn hook(&self) -> &Self::Hook;

    /// Called when an agent event occurs. Default: no-op.
    fn on_agent_event(&self, _agent: &str, _conversation_id: u64, _event: &AgentEvent) {}

    /// Subscribe to agent events. Returns `None` if event broadcasting
    /// is not supported.
    fn subscribe_events(&self) -> Option<broadcast::Receiver<message::AgentEventMsg>> {
        None
    }

    /// Collect layered instructions (e.g. `Crab.md` files) for the
    /// given working directory.
    fn discover_instructions(&self, _cwd: &Path) -> Option<String> {
        None
    }

    /// Effective working directory for a conversation. Defaults to the
    /// process CWD.
    fn effective_cwd(&self, _conversation_id: u64) -> PathBuf {
        std::env::current_dir().unwrap_or_default()
    }
}

/// Dispatch a tool call through an Env's hook. Utility for Env
/// implementors building their ToolDispatcher impl.
pub fn dispatch_tool<'a, E: Env>(
    env: &'a E,
    name: &'a str,
    args: &'a str,
    agent: &'a str,
    sender: &'a str,
    conversation_id: Option<u64>,
) -> ToolFuture<'a> {
    let call = ToolDispatch {
        args: args.to_owned(),
        agent: agent.to_owned(),
        sender: sender.to_owned(),
        conversation_id,
    };

    match env.hook().dispatch(name, call) {
        Some(fut) => fut,
        None => Box::pin(async move { Err(format!("tool not registered: {name}")) }),
    }
}

impl Env for () {
    type Hook = ();

    fn hook(&self) -> &() {
        &()
    }
}

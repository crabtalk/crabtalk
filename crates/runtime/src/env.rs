//! Env — trait for node-specific capabilities and tool dispatch.
//!
//! The runtime engine talks to a single `Env` implementation. The node
//! crate provides [`NodeEnv`] which bundles event broadcasting,
//! instruction discovery, and a composite Harness. Tests use `()`.

use crate::Harness;
use crate::{AgentEvent, ToolDispatch, ToolFuture};
use store::AgentId;
use tokio::sync::broadcast;

/// The runtime environment — combines server capabilities with tool dispatch.
///
/// Each node/binary defines one implementation that wires together
/// the composite hook, event broadcasting, CWD management, and
/// instruction discovery.
pub trait Env: Send + Sync + 'static {
    /// The composite hook providing tool schemas, dispatch, and lifecycle.
    type Hook: Harness;

    /// Access the composite hook.
    fn hook(&self) -> &Self::Hook;

    /// Called when an agent event occurs. Default: no-op.
    ///
    /// `ephemeral` marks events from an anonymous, unpersisted turn —
    /// `session_id` is then a caller-supplied correlation id, not a
    /// real session.
    fn on_agent_event(
        &self,
        _agent: &AgentId,
        _session_id: u64,
        _ephemeral: bool,
        _event: &AgentEvent,
    ) {
    }

    /// Subscribe to agent events. Returns `None` if event broadcasting
    /// is not supported.
    fn subscribe_events(&self) -> Option<broadcast::Receiver<proto::AgentEventMsg>> {
        None
    }
}

/// Dispatch a tool call through an Env's hook. Utility for Env
/// implementors building their ToolDispatcher impl.
pub fn dispatch_tool<'a, E: Env>(
    env: &'a E,
    name: &'a str,
    args: &'a str,
    agent: &'a AgentId,
    sender: &'a str,
    session_id: Option<u64>,
    call_id: &'a str,
) -> ToolFuture<'a> {
    let call = ToolDispatch {
        args: args.to_owned(),
        agent: *agent,
        sender: sender.to_owned(),
        session_id,
        call_id: call_id.to_owned(),
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

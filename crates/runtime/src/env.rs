//! Env — trait for node-specific capabilities and tool dispatch.
//!
//! The runtime engine talks to a single `Env` implementation.
//! `crabtalk`'s `SystemEnv` is the shipped one, bundling event
//! broadcasting and a composite Harness. Tests use `()`.

use crate::{AgentEvent, Harness};
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

impl Env for () {
    type Hook = ();

    fn hook(&self) -> &() {
        &()
    }
}

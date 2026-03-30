//! Backend — trait for server-specific tool dispatch.
//!
//! The runtime crate defines this trait. The daemon implements it to provide
//! `ask_user`, `delegate`, and per-session CWD resolution. Embedded users
//! get [`NoBackend`] with no-op defaults.

use std::path::PathBuf;

/// Trait for server-specific tool dispatch that the runtime cannot handle locally.
pub trait Backend: Send + Sync {
    /// Handle `ask_user` — block until user replies.
    fn dispatch_ask_user(
        &self,
        args: &str,
        session_id: Option<u64>,
    ) -> impl std::future::Future<Output = String> + Send {
        let _ = (args, session_id);
        async { "ask_user is not available in this runtime mode".to_owned() }
    }

    /// Handle `delegate` — spawn sub-agent tasks.
    fn dispatch_delegate(
        &self,
        args: &str,
        agent: &str,
    ) -> impl std::future::Future<Output = String> + Send {
        let _ = (args, agent);
        async { "delegate is not available in this runtime mode".to_owned() }
    }

    /// Resolve the working directory for a session.
    /// Returns `None` to fall back to the runtime's base cwd.
    fn session_cwd(&self, _session_id: u64) -> Option<PathBuf> {
        None
    }

    /// Called when an agent event occurs. The daemon uses this to broadcast
    /// protobuf events to console subscribers. Default: no-op.
    fn on_agent_event(&self, _agent: &str, _session_id: u64, _event: &wcore::AgentEvent) {}

    /// Handle a tool call not matched by the built-in dispatch table.
    /// Downstream backends override this to inject private tools.
    fn dispatch_custom_tool(
        &self,
        name: &str,
        _args: &str,
        _agent: &str,
        _session_id: Option<u64>,
    ) -> impl std::future::Future<Output = String> + Send {
        async move { format!("tool not available: {name}") }
    }
}

/// No-op backend for embedded use.
pub struct NoBackend;

impl Backend for NoBackend {}

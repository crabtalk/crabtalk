//! Host — trait for server-specific capabilities.
//!
//! The runtime crate defines this trait. The daemon implements it to provide
//! event broadcasting, MCP bridge management, and layered instruction
//! discovery. Embedded users get [`NoHost`] with no-op defaults.
//!
//! Tool dispatch and session state (CWD overrides, pending asks) are NOT
//! part of this trait — they use shared state captured by handler factories.

use std::path::Path;

/// Trait for server-specific capabilities that the runtime cannot
/// provide locally.
pub trait Host: Send + Sync + Clone {
    /// Called when an agent event occurs. The daemon uses this to broadcast
    /// protobuf events to console subscribers. Default: no-op.
    fn on_agent_event(&self, _agent: &str, _conversation_id: u64, _event: &wcore::AgentEvent) {}

    /// Deliver a user reply to a pending `ask_user` tool call.
    /// Returns `true` if a pending ask was found and resolved.
    fn reply_to_ask(
        &self,
        _session: u64,
        _content: String,
    ) -> impl std::future::Future<Output = anyhow::Result<bool>> + Send {
        async { Ok(false) }
    }

    /// Subscribe to agent events. Returns `None` if event broadcasting
    /// is not supported by this host.
    fn subscribe_events(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<wcore::protocol::message::AgentEventMsg>> {
        None
    }

    /// Collect layered instructions (e.g. `Crab.md` files) for the
    /// given working directory. Called from `on_before_run` once per
    /// turn, so hosts can surface per-project or per-workspace
    /// guidance to the agent without the runtime itself walking the
    /// filesystem.
    fn discover_instructions(&self, _cwd: &Path) -> Option<String> {
        None
    }

    /// List connected MCP servers with their tool names.
    /// Used by `on_build_agent` to inject available tools into the prompt.
    fn mcp_servers(&self) -> Vec<(String, Vec<String>)> {
        Vec::new()
    }

    /// Return MCP tool schemas for registration in the tool registry.
    fn mcp_tools(&self) -> Vec<wcore::model::Tool> {
        Vec::new()
    }

    /// Inject the MCP handler after async construction. The handler is
    /// type-erased so the runtime crate doesn't depend on the daemon's
    /// MCP types. DaemonHost downcasts; other hosts ignore.
    fn set_mcp(&mut self, _handler: std::sync::Arc<dyn std::any::Any + Send + Sync>) {}
}

/// No-op host for embedded use.
#[derive(Clone)]
pub struct NoHost;

impl Host for NoHost {}

//! Host — trait for server-specific capabilities.
//!
//! The runtime crate defines this trait. The daemon implements it to provide
//! event broadcasting, MCP bridge management, and layered instruction
//! discovery. Embedded users get [`NoHost`] with no-op defaults.
//!
//! Tool dispatch and session state (CWD overrides, pending asks) are NOT
//! part of this trait — they use shared state captured by handler factories.

use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use wcore::protocol::message;

/// Trait for server-specific capabilities that the runtime cannot
/// provide locally: event broadcasting and instruction discovery.
pub trait Host: Send + Sync + Clone {
    /// Called when an agent event occurs. The daemon uses this to broadcast
    /// protobuf events to console subscribers. Default: no-op.
    fn on_agent_event(&self, _agent: &str, _conversation_id: u64, _event: &wcore::AgentEvent) {}

    /// Subscribe to agent events. Returns `None` if event broadcasting
    /// is not supported by this host.
    fn subscribe_events(&self) -> Option<broadcast::Receiver<message::AgentEventMsg>> {
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

    /// Effective working directory for a conversation. Defaults to the
    /// process CWD. Hosts that support per-conversation CWD overrides
    /// (e.g. for delegated tasks) resolve them here.
    fn effective_cwd(&self, _conversation_id: u64) -> PathBuf {
        std::env::current_dir().unwrap_or_default()
    }
}

impl Host for () {}

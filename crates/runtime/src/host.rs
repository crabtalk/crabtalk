//! Host — trait for server-specific tool dispatch.
//!
//! The runtime crate defines this trait. The daemon implements it to provide
//! `ask_user`, `delegate`, per-conversation CWD resolution, and layered
//! instruction discovery. Embedded users get [`NoHost`] with no-op defaults.

use std::path::{Path, PathBuf};

/// Trait for server-specific tool dispatch that the runtime cannot handle locally.
pub trait Host: Send + Sync + Clone {
    /// Handle `ask_user` — block until user replies.
    ///
    /// Returns `Ok` for a normal reply, `Err` for a failure (not available,
    /// timeout, cancelled, invalid args).
    fn dispatch_ask_user(
        &self,
        args: &str,
        conversation_id: Option<u64>,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        let _ = (args, conversation_id);
        async { Err("ask_user is not available in this runtime mode".to_owned()) }
    }

    /// Handle `delegate` — spawn sub-agent tasks.
    ///
    /// Returns `Ok` for successful delegation output, `Err` for failure.
    fn dispatch_delegate(
        &self,
        args: &str,
        agent: &str,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        let _ = (args, agent);
        async { Err("delegate is not available in this runtime mode".to_owned()) }
    }

    /// Resolve the working directory for a conversation.
    /// Returns `None` to fall back to the runtime's base cwd.
    fn conversation_cwd(&self, _conversation_id: u64) -> Option<PathBuf> {
        None
    }

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

    /// Set the working directory override for a conversation.
    fn set_conversation_cwd(
        &self,
        _conversation: u64,
        _cwd: PathBuf,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }

    /// Clear all per-conversation state (pending asks, CWD overrides).
    fn clear_conversation_state(
        &self,
        _conversation: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
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
    ///
    /// Default: `None`. The daemon walks `cwd` upward and merges with
    /// a global file under `~/.crabtalk/`; embedded users who want the
    /// same behaviour override this.
    fn discover_instructions(&self, _cwd: &Path) -> Option<String> {
        None
    }

    /// Handle the `mcp` meta-tool: list/call MCP server tools.
    ///
    /// `allowed_mcps` is the agent's MCP scope (empty = unrestricted).
    /// The host owns the MCP bridge and handles subprocess/HTTP I/O.
    fn dispatch_mcp(
        &self,
        _args: &str,
        _allowed_mcps: &[String],
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        async { Err("mcp is not available in this runtime mode".to_owned()) }
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

    /// Handle a tool call not matched by the built-in dispatch table.
    /// Downstream hosts override this to inject private tools.
    ///
    /// Returns `Ok` on success, `Err` for unknown tools or failures.
    fn dispatch_custom_tool(
        &self,
        name: &str,
        _args: &str,
        _agent: &str,
        _conversation_id: Option<u64>,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        async move { Err(format!("tool not available: {name}")) }
    }
}

/// No-op host for embedded use.
#[derive(Clone)]
pub struct NoHost;

impl Host for NoHost {}

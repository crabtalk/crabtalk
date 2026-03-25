//! Crabtalk MCP bridge — connects to MCP servers and dispatches tool calls.
//!
//! The [`McpBridge`] manages connections to MCP servers via the rmcp SDK,
//! converts tool definitions to crabtalk-core format, and routes tool calls.
//! [`McpHandler`] wraps the bridge with hot-reload and config persistence.
//! `register_tools` registers only tool schemas — dispatch is handled
//! statically by the daemon event loop via [`McpBridge::call`].

use wcore::agent::AsTool;
pub use {bridge::McpBridge, config::McpServerConfig, handler::McpHandler};

pub mod auth;
mod bridge;
pub mod config;
mod handler;
pub(crate) mod tool;

impl McpHandler {
    /// Register the `mcp` tool schema into the registry.
    pub fn register_tools(&self, registry: &mut wcore::ToolRegistry) {
        registry.insert(tool::Mcp::as_tool());
    }
}

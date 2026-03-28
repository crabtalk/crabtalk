//! Crabtalk MCP bridge — connects to MCP servers and dispatches tool calls.

use wcore::agent::AsTool;
pub use {bridge::McpBridge, config::McpServerConfig, handler::McpHandler};

mod bridge;
mod client;
pub mod config;
mod handler;
pub mod tool;

impl McpHandler {
    /// Register the `mcp` tool schema into the registry.
    pub fn register_tools(&self, registry: &mut wcore::ToolRegistry) {
        registry.insert(tool::Mcp::as_tool());
    }
}

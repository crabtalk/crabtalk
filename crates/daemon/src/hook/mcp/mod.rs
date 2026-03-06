//! Walrus MCP bridge — connects to MCP servers and dispatches tool calls.
//!
//! The [`McpBridge`] manages connections to MCP servers via the rmcp SDK,
//! converts tool definitions to walrus-core format, and routes tool calls.
//! [`McpHandler`] wraps the bridge with hot-reload and config persistence.
//! `on_register_tools` registers only tool schemas — dispatch is handled
//! statically by the daemon event loop via [`McpBridge::call`].

pub use {bridge::McpBridge, handler::McpHandler};

mod bridge;
mod handler;

impl wcore::Hook for McpHandler {
    fn on_register_tools(
        &self,
        registry: &mut wcore::ToolRegistry,
    ) -> impl std::future::Future<Output = ()> + Send {
        let bridge = self.try_bridge();
        async move {
            let Some(bridge) = bridge else { return };
            for tool in bridge.tools().await {
                registry.insert(tool);
            }
        }
    }
}

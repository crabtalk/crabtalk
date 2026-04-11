//! Node-specific tool handler factories.
//!
//! Delegate and MCP stay in node because they depend on node types
//! (NodeEventSender, McpHandler). OS, memory, skill, and ask_user
//! tools live in the `tools` crate.

pub mod delegate;
pub mod mcp;

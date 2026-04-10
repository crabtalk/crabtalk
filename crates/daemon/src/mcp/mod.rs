//! MCP bridge — subprocess and HTTP transport for MCP tool servers.
//!
//! Daemon-owned because MCP involves spawning child processes, opening
//! HTTP connections, and scanning the filesystem for port files. Runtime
//! accesses MCP through the [`Host`](runtime::host::Host) trait.

pub use {bridge::McpBridge, config::McpServerConfig, handler::McpHandler};

mod bridge;
mod client;
pub mod config;
pub mod dispatch;
mod handler;

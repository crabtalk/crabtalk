//! MCP tool schema — the `mcp` meta-tool that agents call.
//!
//! The MCP bridge, client, and handler live in the daemon crate (they
//! do I/O). Runtime only defines the tool schema and dispatches through
//! the [`Host`](crate::host::Host) trait.

pub mod config;
pub mod tool;

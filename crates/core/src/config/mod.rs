//! Shared configuration types used across crates.

pub mod mcp;
pub mod provider;

pub use mcp::McpServerConfig;
pub use provider::{ApiStandard, ProviderDef};

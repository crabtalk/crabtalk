//! Shared configuration types used across crates.

pub mod mcp;
pub mod provider;
pub mod service;

pub use mcp::McpServerConfig;
pub use provider::{ApiStandard, ProviderDef};
pub use service::ServiceConfig;

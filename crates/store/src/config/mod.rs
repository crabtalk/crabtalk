//! Shared configuration types used across crates.

pub mod cache;
pub mod crabtalk;
mod env;
pub mod harness;
pub mod hooks;
pub mod llm;
pub mod mcp;
pub mod system;

pub use cache::CacheConfig;
pub use crabtalk::Config;
pub use harness::{HarnessConfig, Root};
pub use hooks::{HooksConfig, MemoryConfig};
pub use llm::LlmConfig;
pub use mcp::{McpConfig, McpServerConfig};
pub use system::TasksConfig;

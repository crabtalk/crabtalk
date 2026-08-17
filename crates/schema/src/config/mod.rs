//! Shared configuration types used across crates.

pub mod crabtalk;
pub mod harness;
pub mod hooks;
pub mod llm;
pub mod manifest;
pub mod mcp;
pub mod system;

pub use crabtalk::Config;
pub use harness::HarnessConfig;
pub use hooks::{HooksConfig, MemoryConfig};
pub use llm::LlmConfig;
pub use manifest::{
    PackageMeta, ResolvedDirs, Setup, external_source_name, load_agents_dir, load_agents_dirs,
    repo_slug, resolve_dirs,
};
pub use mcp::{McpConfig, McpServerConfig};
pub use system::TasksConfig;

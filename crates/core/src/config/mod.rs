//! Shared configuration types used across crates.

pub mod crabtalk;
pub mod hooks;
pub mod llm;
pub mod manifest;
pub mod mcp;
pub mod system;

pub use crabtalk::Config;
pub use hooks::{HooksConfig, MemoryConfig};
pub use llm::LlmConfig;
pub use manifest::{
    PackageMeta, ResolvedDirs, Setup, check_skill_conflicts, external_source_name, load_agents_dir,
    load_agents_dirs, repo_slug, resolve_dirs, scan_skill_names,
};
pub use mcp::McpServerConfig;
pub use system::TasksConfig;

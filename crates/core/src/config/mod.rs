//! Shared configuration types used across crates.

pub mod daemon;
pub mod hooks;
pub mod manifest;
pub mod mcp;
pub mod provider;
pub mod system;

pub use daemon::{DaemonConfig, validate_providers};
pub use hooks::{BashConfig, HooksConfig, MemoryConfig};
pub use manifest::{
    DisabledItems, ManifestConfig, PackageMeta, ResolvedManifest, Setup, check_skill_conflicts,
    external_source_name, filter_disabled_external, load_agents_dir, load_agents_dirs, repo_slug,
    resolve_manifests, scan_skill_names,
};
pub use mcp::McpServerConfig;
pub use provider::{ApiStandard, PROVIDER_PRESETS, ProviderDef, ProviderPreset};
pub use system::{SystemConfig, TasksConfig};

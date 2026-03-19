//! Service configuration types for `[services.*]` in `crab.toml`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Kind of managed service.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Extension service — speaks Crabtalk Extension protocol over UDS.
    #[default]
    Extension,
    /// Gateway service — speaks existing crabtalk protocol (e.g. Telegram).
    Gateway,
}

/// Restart policy for a managed service.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Never restart on exit.
    #[default]
    Never,
    /// Restart only on non-zero exit.
    OnFailure,
    /// Always restart.
    Always,
}

/// Per-service configuration from `[services.<name>]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Human-readable description (for hub display).
    #[serde(default)]
    pub description: Option<String>,
    /// Service kind.
    #[serde(default)]
    pub kind: ServiceKind,
    /// Cargo package name (e.g. "crabtalk-memory"). Used as binary name and for
    /// `cargo install` during hub installation.
    #[serde(rename = "crate", alias = "krate")]
    pub krate: String,
    /// Restart policy.
    #[serde(default)]
    pub restart: RestartPolicy,
    /// Whether the service is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Environment variables injected into the child process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Opaque service-specific configuration (forwarded via extension Configure).
    #[serde(default)]
    pub config: Value,
}

fn default_true() -> bool {
    true
}

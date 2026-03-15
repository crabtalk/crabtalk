//! Service configuration types for `[services.*]` in `walrus.toml`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Kind of managed service.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Hook service — speaks WHS protocol over UDS.
    #[default]
    Hook,
    /// Client service — speaks existing walrus protocol (gateways).
    Client,
    /// Arbitrary process — no walrus protocol (e.g. llama-server).
    Process,
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

/// Install instructions for a service binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    /// Command to execute (e.g. "cargo", "curl").
    pub command: String,
    /// Arguments to pass to the command (e.g. ["install", "walrus-memory"]).
    #[serde(default)]
    pub args: Vec<String>,
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
    /// Command to execute (binary name or path).
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Install instructions (hub install-time only, not written to walrus.toml).
    #[serde(default)]
    pub install: Option<InstallConfig>,
    /// Restart policy.
    #[serde(default)]
    pub restart: RestartPolicy,
    /// Whether the service is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Opaque service-specific configuration (forwarded via WHS Configure).
    #[serde(default)]
    pub config: Value,
}

fn default_true() -> bool {
    true
}

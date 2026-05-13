//! Per-agent hook configuration — bash deny rules, memory recall
//! tuning. Each agent owns its own `HooksConfig` directly on
//! [`crate::AgentConfig`]; there is no global override.

use serde::{Deserialize, Serialize};

/// Per-agent hook configuration.
///
/// OS-tool configuration used to live here under `bash`, but OS tool
/// execution moved entirely client-side; the daemon no longer enforces
/// any bash policy.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HooksConfig {
    /// Memory hook configuration (`hooks.memory` under an agent).
    pub memory: MemoryConfig,
}

/// Built-in memory configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Maximum entries returned by auto-recall (default 5).
    pub recall_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { recall_limit: 5 }
    }
}

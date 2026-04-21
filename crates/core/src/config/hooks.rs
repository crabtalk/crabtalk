//! Hook subsystem configuration — defaults for bash and memory hooks.

use serde::{Deserialize, Serialize};

/// Top-level `[hooks]` configuration. Defaults for built-in hooks.
///
/// Per-scope overrides live in mutable Storage; these values are the
/// fallback when no override is set.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HooksConfig {
    /// Bash tool defaults (`[hooks.bash]`).
    pub bash: BashConfig,
    /// Built-in memory defaults (`[hooks.memory]`).
    pub memory: MemoryConfig,
}

/// Bash tool configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BashConfig {
    /// Disable the bash tool entirely.
    pub disabled: bool,
    /// Reject commands containing any of these strings (e.g. `".ssh"`).
    pub deny: Vec<String>,
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

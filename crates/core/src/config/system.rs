//! System subsystem — default agent and task executor configuration.

use crate::config::hooks::{BashConfig, MemoryConfig};
use serde::{Deserialize, Serialize};

/// Top-level `[system]` configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SystemConfig {
    /// The default system agent config (model, thinking, etc.).
    pub crab: crate::AgentConfig,
    /// Task executor pool configuration (`[system.tasks]`).
    pub tasks: TasksConfig,
    /// **Deprecated**: moved to `[hooks.bash]`. Captured here only for
    /// one-release migration; consumers must read from `DaemonConfig::hooks`.
    #[serde(rename = "bash", skip_serializing)]
    pub legacy_bash: Option<BashConfig>,
    /// **Deprecated**: moved to `[hooks.memory]`. See `legacy_bash` above.
    #[serde(rename = "memory", skip_serializing)]
    pub legacy_memory: Option<MemoryConfig>,
}

/// Task executor pool configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TasksConfig {
    /// Maximum number of concurrently InProgress tasks (default 4).
    pub max_concurrent: usize,
    /// Maximum number of tasks returned by queries (default 16).
    pub viewable_window: usize,
    /// Per-task execution timeout in seconds (default 300).
    pub task_timeout: u64,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            viewable_window: 16,
            task_timeout: 300,
        }
    }
}

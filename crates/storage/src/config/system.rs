//! Task executor pool configuration.

use serde::{Deserialize, Serialize};

/// Task executor pool configuration (`[tasks]` in `config.toml`).
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

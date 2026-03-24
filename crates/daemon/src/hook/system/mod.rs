//! System subsystem — task executor and memory configuration.
//!
//! Groups `[system.tasks]` and `[system.memory]` config under a single
//! `SystemConfig` struct. Task registry and tool dispatch live in `task/`.

use serde::{Deserialize, Serialize};

pub mod ask_user;
pub mod memory;
pub mod session;
pub mod task;

/// Top-level `[system]` configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SystemConfig {
    /// The default system agent config (model, heartbeat, thinking).
    pub crab: wcore::AgentConfig,
    /// Task executor pool configuration (`[system.tasks]`).
    pub tasks: TasksConfig,
    /// Built-in memory configuration (`[system.memory]`).
    pub memory: MemoryConfig,
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

/// Built-in memory configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Maximum entries returned by auto-recall (default 5).
    pub recall_limit: usize,
    /// Whether the agent can edit Crab.md via the soul tool (default true).
    pub soul_editable: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            recall_limit: 5,
            soul_editable: true,
        }
    }
}

//! Memory subsystem configuration.

use serde::{Deserialize, Serialize};

/// Memory subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Additional entity types beyond the framework defaults.
    pub entities: Vec<String>,
    /// Additional relation types beyond the framework defaults.
    pub relations: Vec<String>,
    /// Default limit for `connections` traversal results (default: 20, max: 100).
    pub connections: usize,
    /// Enable automatic memory recall before each agent run (default: true).
    #[serde(default = "default_true")]
    pub auto_recall: bool,
}

fn default_true() -> bool {
    true
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            entities: Vec::new(),
            relations: Vec::new(),
            connections: 20,
            auto_recall: true,
        }
    }
}

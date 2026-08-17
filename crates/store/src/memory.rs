//! A named unit of remembered context.

use serde::{Deserialize, Serialize};

/// One memory entry.
///
/// `kind` distinguishes what wrote it — `note` for the agent's own
/// `remember`, `archive` for a compaction summary — so a recall can tell
/// a fact the agent chose to keep from a transcript it was made to
/// shed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub name: String,
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

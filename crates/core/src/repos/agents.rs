//! Agent repository trait.
//!
//! [`AgentRepo`] abstracts agent config + prompt persistence. The repo
//! owns its encoding (TOML manifest + markdown prompt on the fs impl).

use crate::{AgentConfig, AgentId};
use anyhow::Result;

/// Persistence backend for agent definitions.
///
/// Implementations own the encoding of agent configs and prompts. The
/// trait speaks domain types only — `AgentConfig` and `AgentId`.
pub trait AgentRepo: Send + Sync + 'static {
    /// List all persisted agent configs (with prompts loaded).
    fn list(&self) -> Result<Vec<AgentConfig>>;

    /// Load a single agent by ULID.
    fn load(&self, id: &AgentId) -> Result<Option<AgentConfig>>;

    /// Load a single agent by name.
    fn load_by_name(&self, name: &str) -> Result<Option<AgentConfig>>;

    /// Create or replace an agent. The prompt is stored separately from
    /// the config in the fs impl but arrives as a single call here.
    fn upsert(&self, config: &AgentConfig, prompt: &str) -> Result<()>;

    /// Delete an agent by ULID. Returns `true` if it existed.
    fn delete(&self, id: &AgentId) -> Result<bool>;

    /// Rename an agent. The ULID stays stable.
    fn rename(&self, id: &AgentId, new_name: &str) -> Result<bool>;
}

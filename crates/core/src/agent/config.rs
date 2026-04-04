//! Agent configuration.
//!
//! [`AgentConfig`] is a serializable struct holding all agent parameters.
//! Used by [`super::AgentBuilder`] to construct an [`super::Agent`].

use crate::model::ToolChoice;
use serde::{Deserialize, Serialize};

/// Default maximum iterations for agent execution.
const DEFAULT_MAX_ITERATIONS: usize = 16;

/// Default compact threshold in estimated tokens (~100k).
const DEFAULT_COMPACT_THRESHOLD: usize = 100_000;

/// Default max byte length for tool results during compaction.
const DEFAULT_COMPACT_TOOL_MAX_LEN: usize = 1024;

/// Serializable agent configuration.
///
/// Contains all parameters for an agent: identity, system prompt, model,
/// iteration limits, and delegation scope. Used both as the
/// TOML deserialization target and the runtime agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent identifier. Derived from TOML key, not stored in TOML.
    #[serde(skip)]
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// System prompt sent before each LLM request. Loaded from .md file.
    #[serde(skip)]
    pub system_prompt: String,
    /// Model to use from the registry. None = registry's active/default.
    #[serde(default)]
    pub model: Option<String>,
    /// Maximum iterations before stopping.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Controls which tool the model calls. Defaults to `Auto`.
    #[serde(default)]
    pub tool_choice: ToolChoice,
    /// Whether to enable thinking/reasoning mode.
    #[serde(default)]
    pub thinking: bool,
    /// Agents this agent can delegate to via spawn_task. Empty = no delegation.
    #[serde(default)]
    pub members: Vec<String>,
    /// Skill names this agent can access. Empty = all skills (crabtalk default).
    #[serde(default)]
    pub skills: Vec<String>,
    /// MCP server names this agent can access. Empty = all MCPs (crabtalk default).
    #[serde(default)]
    pub mcps: Vec<String>,
    /// Computed tool whitelist. Empty = all tools. Not serialized.
    #[serde(skip)]
    pub tools: Vec<String>,
    /// Token count threshold for automatic context compaction.
    /// When history exceeds this, the agent compacts automatically.
    /// None = disabled. Defaults to 100_000.
    #[serde(default = "default_compact_threshold")]
    pub compact_threshold: Option<usize>,
    /// Max byte length to keep from tool-role messages when compacting.
    /// Longer results are truncated before sending to the compaction LLM.
    #[serde(default = "default_compact_tool_max_len")]
    pub compact_tool_max_len: usize,
}

fn default_max_iterations() -> usize {
    DEFAULT_MAX_ITERATIONS
}

fn default_compact_threshold() -> Option<usize> {
    Some(DEFAULT_COMPACT_THRESHOLD)
}

fn default_compact_tool_max_len() -> usize {
    DEFAULT_COMPACT_TOOL_MAX_LEN
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            system_prompt: String::new(),
            model: None,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            tool_choice: ToolChoice::Auto,
            thinking: false,
            members: Vec::new(),
            skills: Vec::new(),
            mcps: Vec::new(),
            tools: Vec::new(),
            compact_threshold: default_compact_threshold(),
            compact_tool_max_len: DEFAULT_COMPACT_TOOL_MAX_LEN,
        }
    }
}

impl AgentConfig {
    /// Create a new config with the given name and defaults for everything else.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set the system prompt.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Set the description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set the model to use from the registry.
    pub fn model(mut self, name: impl Into<String>) -> Self {
        self.model = Some(name.into());
        self
    }

    /// Enable or disable thinking/reasoning mode.
    pub fn thinking(mut self, enabled: bool) -> Self {
        self.thinking = enabled;
        self
    }
}

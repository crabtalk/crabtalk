//! Agent configuration.
//!
//! [`AgentConfig`] is a serializable struct holding all agent parameters.
//! Used by [`super::AgentBuilder`] to construct an [`super::Agent`].

use crate::model::ToolChoice;
use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// Default maximum iterations for agent execution.
const DEFAULT_MAX_ITERATIONS: usize = 16;

/// Serializable agent configuration.
///
/// Contains all parameters for an agent: identity, system prompt, model,
/// and iteration limits. All registered tools are available to every agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent identifier.
    pub name: CompactString,
    /// Human-readable description.
    pub description: CompactString,
    /// System prompt sent before each LLM request.
    pub system_prompt: String,
    /// Model to use from the registry. None = registry's active/default.
    pub model: Option<CompactString>,
    /// Maximum iterations before stopping.
    pub max_iterations: usize,
    /// Controls which tool the model calls.
    pub tool_choice: ToolChoice,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: CompactString::default(),
            description: CompactString::default(),
            system_prompt: String::new(),
            model: None,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            tool_choice: ToolChoice::Auto,
        }
    }
}

impl AgentConfig {
    /// Create a new config with the given name and defaults for everything else.
    pub fn new(name: impl Into<CompactString>) -> Self {
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
    pub fn description(mut self, desc: impl Into<CompactString>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set the model to use from the registry.
    pub fn model(mut self, name: impl Into<CompactString>) -> Self {
        self.model = Some(name.into());
        self
    }
}

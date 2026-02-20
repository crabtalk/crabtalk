//! Configuration for a chat

use crate::{Tool, ToolChoice};
use serde::{Deserialize, Serialize};

/// LLM configuration
pub trait Config: From<General> + Sized + Clone {
    /// Create a new configuration with tools
    fn with_tools(self, tools: Vec<Tool>) -> Self;

    /// Create a new configuration with tool choice
    ///
    /// This should be used for per-message level.
    fn with_tool_choice(self, tool_choice: ToolChoice) -> Self;
}

/// Chat configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct General {
    /// The model to use
    pub model: String,

    /// Whether to enable thinking
    pub think: bool,

    /// The tools to use
    pub tools: Option<Vec<Tool>>,

    /// Whether to return the usage information in stream mode
    pub usage: bool,
}

impl General {
    /// Create a new configuration
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            think: false,
            tools: None,
            usage: false,
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self {
            model: "deepseek-chat".into(),
            think: false,
            tools: None,
            usage: false,
        }
    }
}

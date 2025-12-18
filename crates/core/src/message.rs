//! Turbofish LLM message

use derive_more::Display;

/// A message in the chat
#[derive(Debug, Clone)]
pub struct Message {
    /// The role of the message
    pub role: Role,

    /// The content of the message
    pub content: String,
}

impl Message {
    /// Create a new system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Create a new user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Create a new assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }

    /// Create a new tool message
    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
        }
    }
}

/// The role of a message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
pub enum Role {
    /// The user role
    #[display("user")]
    User,
    /// The assistant role
    #[display("assistant")]
    Assistant,
    /// The system role
    #[display("system")]
    System,
    /// The tool role
    #[display("tool")]
    Tool,
}

//! Streaming response abstractions for the unified LLM Interfaces

use crate::{FinishReason, Role, tool::ToolCall};
use serde::Deserialize;

/// A streaming chat completion chunk
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StreamChunk {
    /// A unique identifier for the chat completion
    pub id: String,

    /// The object type, always "chat.completion.chunk"
    pub object: String,

    /// Unix timestamp (in seconds) of when the chunk was created
    pub created: u64,

    /// The model used for the completion
    pub model: String,

    /// Backend configuration identifier
    pub system_fingerprint: Option<String>,

    /// The list of completion choices (with delta content)
    pub choices: Vec<StreamChoice>,

    /// Token usage statistics (only in final chunk)
    pub usage: Option<crate::Usage>,
}

impl StreamChunk {
    /// Create a new tool chunk
    pub fn tool(calls: &[ToolCall]) -> Self {
        Self {
            choices: vec![StreamChoice {
                delta: Delta {
                    tool_calls: Some(calls.to_vec()),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Get the content of the first choice
    pub fn content(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.delta.content.as_deref())
            .filter(|s| !s.is_empty())
    }

    /// Get the reasoning content of the first choice
    pub fn reasoning_content(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.delta.reasoning_content.as_deref())
            .filter(|s| !s.is_empty())
    }

    /// Get the tool calls of the first choice
    pub fn tool_calls(&self) -> Option<&[ToolCall]> {
        self.choices
            .first()
            .and_then(|choice| choice.delta.tool_calls.as_deref())
    }

    /// Get the reason the model stopped generating
    pub fn reason(&self) -> Option<&FinishReason> {
        self.choices
            .first()
            .and_then(|choice| choice.finish_reason.as_ref())
    }
}

/// A completion choice in a streaming response
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StreamChoice {
    /// The index of this choice in the list
    pub index: u32,

    /// The delta content for this chunk
    pub delta: Delta,

    /// The reason the model stopped generating
    pub finish_reason: Option<crate::FinishReason>,

    /// Log probability information
    pub logprobs: Option<crate::LogProbs>,
}

/// Delta content in a streaming response
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Delta {
    /// The role of the message author
    pub role: Option<Role>,

    /// The content delta
    pub content: Option<String>,

    /// The reasoning content delta (for deepseek-reasoner model)
    pub reasoning_content: Option<String>,

    /// Tool calls delta
    pub tool_calls: Option<Vec<ToolCall>>,
}

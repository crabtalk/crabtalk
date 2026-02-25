//! Streaming response abstractions for the unified LLM Interfaces

use crate::{
    FinishReason,
    response::{Choice, CompletionMeta, Delta},
    tool::ToolCall,
};
use serde::Deserialize;

/// A streaming chat completion chunk
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StreamChunk {
    /// Completion metadata
    #[serde(flatten)]
    pub meta: CompletionMeta,

    /// The list of completion choices (with delta content)
    pub choices: Vec<Choice>,

    /// Token usage statistics (only in final chunk)
    pub usage: Option<crate::Usage>,
}

impl StreamChunk {
    /// Create a new tool chunk
    pub fn tool(calls: &[ToolCall]) -> Self {
        Self {
            choices: vec![Choice {
                delta: Delta {
                    tool_calls: Some(calls.to_vec()),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Create a separator chunk (newline) emitted between tool-call rounds.
    pub fn separator() -> Self {
        Self {
            choices: vec![Choice {
                delta: Delta {
                    content: Some("\n".into()),
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

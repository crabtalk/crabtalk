//! Chat response abstractions for the unified LLM Interfaces

use crate::{Message, Role, tool::ToolCall};
use serde::{Deserialize, Serialize};

/// A chat completion response from the LLM
#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    /// A unique identifier for the chat completion
    pub id: String,

    /// The object type, always "chat.completion"
    pub object: String,

    /// Unix timestamp (in seconds) of when the response was created
    pub created: u64,

    /// The model used for the completion
    pub model: String,

    /// Backend configuration identifier
    pub system_fingerprint: Option<String>,

    /// The list of completion choices
    pub choices: Vec<Choice>,

    /// Token usage statistics
    pub usage: Usage,
}

impl Response {
    pub fn message(&self) -> Option<Message> {
        let choice = self.choices.first()?;
        Some(Message::assistant(
            choice.message.content.clone().unwrap_or_default(),
            choice.message.reasoning_content.clone(),
            choice.message.tool_calls.as_deref(),
        ))
    }

    /// Get the first message from the response
    pub fn content(&self) -> Option<&String> {
        self.choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
    }

    /// Get the first message from the response
    pub fn reasoning(&self) -> Option<&String> {
        self.choices
            .first()
            .and_then(|choice| choice.message.reasoning_content.as_ref())
    }

    /// Get the tool calls from the response
    pub fn tool_calls(&self) -> Option<&[ToolCall]> {
        self.choices
            .first()
            .and_then(|choice| choice.message.tool_calls.as_deref())
    }

    /// Get the reason the model stopped generating
    pub fn reason(&self) -> Option<&FinishReason> {
        self.choices
            .first()
            .and_then(|choice| choice.finish_reason.as_ref())
    }
}

/// A completion choice in a non-streaming response
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    /// The index of this choice in the list
    pub index: u32,

    /// The generated message
    pub message: ResponseMessage,

    /// The reason the model stopped generating
    pub finish_reason: Option<FinishReason>,

    /// Log probability information
    pub logprobs: Option<LogProbs>,
}

/// A message in the response
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseMessage {
    /// The role of the message author (always "assistant")
    pub role: Role,

    /// The content of the message
    pub content: Option<String>,

    /// The reasoning content (for deepseek-reasoner model)
    pub reasoning_content: Option<String>,

    /// Tool calls made by the model
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// The reason the model stopped generating
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The model finished naturally
    Stop,

    /// The model hit the max token limit
    Length,

    /// Content was filtered
    ContentFilter,

    /// The model is making tool calls
    ToolCalls,

    /// Insufficient system resources
    InsufficientSystemResource,
}

/// Token usage statistics
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    /// Number of tokens in the prompt
    pub prompt_tokens: u32,

    /// Number of tokens in the completion
    pub completion_tokens: u32,

    /// Total number of tokens used
    pub total_tokens: u32,

    /// Number of prompt tokens from cache hits
    pub prompt_cache_hit_tokens: Option<u32>,

    /// Number of prompt tokens not in cache
    pub prompt_cache_miss_tokens: Option<u32>,

    /// Detailed breakdown of completion tokens
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

/// Detailed breakdown of completion tokens
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionTokensDetails {
    /// Number of tokens used for reasoning
    pub reasoning_tokens: Option<u32>,
}

/// Log probability information
#[derive(Debug, Clone, Deserialize)]
pub struct LogProbs {
    /// Log probabilities for each token
    pub content: Option<Vec<LogProb>>,
}

/// Log probability for a single token
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogProb {
    /// The token string
    pub token: String,

    /// The log probability of this token
    pub logprob: f64,

    /// Byte representation of the token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,

    /// Top log probabilities for this position
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<Vec<TopLogProb>>,
}

/// Top log probability entry
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TopLogProb {
    /// The token string
    pub token: String,

    /// The log probability
    pub logprob: f64,

    /// Byte representation of the token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

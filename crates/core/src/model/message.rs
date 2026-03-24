//! Turbofish LLM message

use crate::model::{StreamChunk, ToolCall};
pub use crabllm_core::Role;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A message in the chat
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    /// The role of the message
    pub role: Role,

    /// The content of the message
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,

    /// The reasoning content
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reasoning_content: String,

    /// The tool call id
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,

    /// The tool calls
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,

    /// The sender identity (runtime-only, never serialized to providers).
    ///
    /// Convention: empty = local/owner, `"tg:12345"` = Telegram user.
    #[serde(skip)]
    pub sender: String,

    /// Whether this message was auto-injected by the runtime (e.g. recall).
    /// Auto-injected messages are stripped before each new run.
    #[serde(skip)]
    pub auto_injected: bool,
}

impl Message {
    /// Create a new system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            ..Default::default()
        }
    }

    /// Create a new user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            ..Default::default()
        }
    }

    /// Create a new user message with sender identity.
    pub fn user_with_sender(content: impl Into<String>, sender: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            sender: sender.into(),
            ..Default::default()
        }
    }

    /// Create a new assistant message
    pub fn assistant(
        content: impl Into<String>,
        reasoning: Option<String>,
        tool_calls: Option<&[ToolCall]>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            reasoning_content: reasoning.unwrap_or_default(),
            tool_calls: tool_calls.map(|tc| tc.to_vec()).unwrap_or_default(),
            ..Default::default()
        }
    }

    /// Create a new tool message
    pub fn tool(content: impl Into<String>, call: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: call.into(),
            ..Default::default()
        }
    }

    /// Create a new message builder
    pub fn builder(role: Role) -> MessageBuilder {
        MessageBuilder::new(role)
    }

    /// Estimate the number of tokens in this message.
    ///
    /// Uses a simple heuristic: ~4 characters per token.
    pub fn estimate_tokens(&self) -> usize {
        let chars = self.content.len()
            + self.reasoning_content.len()
            + self.tool_call_id.len()
            + self
                .tool_calls
                .iter()
                .map(|tc| tc.function.name.len() + tc.function.arguments.len())
                .sum::<usize>();
        (chars / 4).max(1)
    }
}

/// Estimate total tokens across a slice of messages.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    messages.iter().map(|m| m.estimate_tokens()).sum()
}

/// A builder for messages
pub struct MessageBuilder {
    /// The message
    message: Message,
    /// The tool calls
    calls: BTreeMap<u32, ToolCall>,
}

impl MessageBuilder {
    /// Create a new message builder
    pub fn new(role: Role) -> Self {
        Self {
            message: Message {
                role,
                ..Default::default()
            },
            calls: BTreeMap::new(),
        }
    }

    /// Accept a chunk from the stream
    pub fn accept(&mut self, chunk: &StreamChunk) -> bool {
        if let Some(calls) = chunk.tool_calls() {
            for call in calls {
                let entry = self.calls.entry(call.index).or_default();
                entry.merge(call);
            }
        }

        let mut has_content = false;
        if let Some(content) = chunk.content() {
            self.message.content.push_str(content);
            has_content = true;
        }

        if let Some(reason) = chunk.reasoning_content() {
            self.message.reasoning_content.push_str(reason);
        }

        has_content
    }

    /// Peek at accumulated tool calls with non-empty names.
    /// Returns clones of the current state (args may be partial).
    pub fn peek_tool_calls(&self) -> Vec<ToolCall> {
        self.calls
            .values()
            .filter(|c| !c.function.name.is_empty())
            .cloned()
            .collect()
    }

    /// Build the message
    pub fn build(mut self) -> Message {
        if !self.calls.is_empty() {
            self.message.tool_calls = self.calls.into_values().collect();
        }
        self.message
    }
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: Role::User,
            content: String::new(),
            reasoning_content: String::new(),
            tool_call_id: String::new(),
            tool_calls: Vec::new(),
            sender: String::new(),
            auto_injected: false,
        }
    }
}

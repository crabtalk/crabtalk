//! Turbofish LLM message

use std::collections::BTreeMap;

use crate::{StreamChunk, ToolCall};
use serde::{Deserialize, Serialize};

/// A message in the chat
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Message {
    /// The role of the message
    pub role: Role,

    /// The content of the message
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,

    /// The name of the message
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,

    /// Whether to prefix the message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<bool>,

    /// The reasoning content
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reasoning_content: String,

    /// The tool call id
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,

    /// The tool calls
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
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
            tool_calls: tool_calls.unwrap_or_default().to_vec(),
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

    /// Build the message
    pub fn build(mut self) -> Message {
        if !self.calls.is_empty() {
            self.message.tool_calls = self.calls.into_values().collect();
        }
        self.message
    }
}

/// The role of a message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, Default)]
pub enum Role {
    /// The user role
    #[serde(rename = "user")]
    #[default]
    User,
    /// The assistant role
    #[serde(rename = "assistant")]
    Assistant,
    /// The system role
    #[serde(rename = "system")]
    System,
    /// The tool role
    #[serde(rename = "tool")]
    Tool,
}

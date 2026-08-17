//! One turn of a session, as persisted.
//!
//! The inner `message` is the wire shape sent to providers; the rest is
//! runtime state the session file keeps so a reload can reconstruct it.

use crate::session::MAX_SNIPPET_BYTES;
use crabllm_core::{
    Role, ToolCall,
    anthropic::{Content, ContentBlock, Message},
};
use serde::{Deserialize, Serialize};

/// A single session history entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HistoryEntry {
    /// Which agent produced this assistant message.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,

    /// The sender identity (runtime-only, never serialized).
    #[serde(skip)]
    pub sender: String,

    /// Whether this entry was auto-injected by the runtime (runtime-only).
    /// Auto-injected entries are stripped before each new run and never
    /// persisted as session steps.
    #[serde(skip)]
    pub auto_injected: bool,

    /// The wire-level message sent to providers.
    pub message: Message,
}

impl HistoryEntry {
    /// Create a new system entry.
    pub fn system(content: impl Into<String>) -> Self {
        Self::from_message(Message {
            role: Role::System.as_str().to_string(),
            content: Content::Blocks(vec![ContentBlock::text(content)]),
        })
    }

    /// Create a new user entry.
    pub fn user(content: impl Into<String>) -> Self {
        Self::from_message(Message::user(content))
    }

    /// Create a new user entry with sender identity.
    pub fn user_with_sender(content: impl Into<String>, sender: impl Into<String>) -> Self {
        let mut entry = Self::user(content);
        entry.sender = sender.into();
        entry
    }

    /// Create a new assistant entry from content blocks.
    pub fn assistant(
        content: impl Into<String>,
        reasoning: Option<String>,
        tool_calls: Option<&[ToolCall]>,
    ) -> Self {
        Self::from_message(Message::assistant_parts(
            content,
            reasoning,
            tool_calls.unwrap_or_default(),
        ))
    }

    /// Create a new tool-result entry.
    pub fn tool(
        content: impl Into<String>,
        call_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self::from_message(Message::tool(call_id, name, content))
    }

    /// Wrap an existing `anthropic::Message`.
    pub fn from_message(message: Message) -> Self {
        Self {
            agent: String::new(),
            sender: String::new(),
            auto_injected: false,
            message,
        }
    }

    /// Mark this entry as auto-injected (chainable).
    pub fn auto_injected(mut self) -> Self {
        self.auto_injected = true;
        self
    }

    /// The role of the underlying message.
    pub fn role(&self) -> Role {
        match self.message.role.as_str() {
            "assistant" => Role::Assistant,
            "system" => Role::System,
            _ => Role::User,
        }
    }

    /// The text content of the message, or `""` if absent / empty.
    pub fn text(&self) -> &str {
        self.message.text().unwrap_or("")
    }

    /// The reasoning/thinking content, or empty if absent.
    pub fn reasoning(&self) -> &str {
        self.message.thinking().unwrap_or("")
    }

    /// The tool calls on this entry as ToolCall structs.
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.message.tool_calls()
    }

    /// The tool_use_id from the first ToolResult block, or empty.
    pub fn tool_call_id(&self) -> &str {
        self.message
            .blocks()
            .iter()
            .find_map(|b| match b {
                ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .unwrap_or("")
    }

    /// Project to a `Message` for sending to a provider.
    ///
    /// If this is a guest assistant message (`agent` non-empty and role is
    /// Assistant), wraps the text content in `<from agent="...">` tags so other
    /// agents can distinguish speakers in multi-agent sessions.
    pub fn to_wire_message(&self) -> Message {
        if self.role() != Role::Assistant || self.agent.is_empty() {
            return self.message.clone();
        }
        let mut message = self.message.clone();
        if let Some(blocks) = message.blocks_mut() {
            for block in blocks {
                if let ContentBlock::Text { text, .. } = block {
                    *text = format!("<from agent=\"{}\">\n{}\n</from>", self.agent, text);
                }
            }
        }
        message
    }
}

impl HistoryEntry {
    /// Text and role tag for the search index, or `None` for a message
    /// that must not be indexed.
    ///
    /// The exclusions are the point. Tool results and tool-call
    /// arguments carry credentials often enough that neither belongs in
    /// free text a query can reach — a tool-calling assistant
    /// contributes only the function names. Auto-injected framing is not
    /// the user's words and would match against every session that has
    /// it. The tag feeds the ranking weights, so `assistant_tool` is
    /// distinct from `assistant`.
    pub fn indexable(&self) -> Option<(String, &'static str)> {
        if self.auto_injected {
            return None;
        }
        let role = self.role().clone();
        if !matches!(role, Role::User | Role::Assistant) || self.has_tool_result() {
            return None;
        }
        let text = self.text();
        if !text.is_empty() {
            let tag = if matches!(role, Role::User) {
                "user"
            } else {
                "assistant"
            };
            return Some((text.to_owned(), tag));
        }
        let names: Vec<_> = self
            .tool_calls()
            .iter()
            .map(|tc| tc.function.name.clone())
            .collect();
        if names.is_empty() {
            return None;
        }
        let tag = if matches!(role, Role::User) {
            "user"
        } else {
            "assistant_tool"
        };
        Some((names.join(" "), tag))
    }

    /// The entry's text, cut to [`MAX_SNIPPET_BYTES`] on a character
    /// boundary. The flag says whether anything was dropped.
    pub fn snippet(&self) -> (String, bool) {
        let raw = self.text().to_owned();
        if raw.len() <= MAX_SNIPPET_BYTES {
            return (raw, false);
        }
        let mut end = MAX_SNIPPET_BYTES;
        while end > 0 && !raw.is_char_boundary(end) {
            end -= 1;
        }
        (raw[..end].to_owned(), true)
    }

    /// The function name on a tool call or a tool result.
    pub fn tool_name(&self) -> Option<String> {
        for block in self.message.blocks() {
            match block {
                ContentBlock::ToolResult { name: Some(n), .. } if !n.is_empty() => {
                    return Some(n.clone());
                }
                ContentBlock::ToolUse { name, .. } => return Some(name.clone()),
                _ => {}
            }
        }
        None
    }

    fn has_tool_result(&self) -> bool {
        self.message
            .blocks()
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
    }
}

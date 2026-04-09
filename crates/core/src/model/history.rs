//! `HistoryEntry` — a conversation entry wrapping a wire-level
//! `crabllm_core::Message` plus runtime-only fields.
//!
//! The three runtime-only fields (`sender`, `agent`, `auto_injected`) carry
//! context the wire type cannot: user identity for multi-sender conversations,
//! guest-agent attribution for multi-agent conversations (RFC 0135), and a
//! strip-before-next-run flag for runtime-generated framing (recall results,
//! env blocks, CWD hints).
//!
//! The inner `crabllm_core::Message` is the shape providers see — `HistoryEntry`
//! projects to it in `Agent::build_request`.

use crabllm_core::{Message, Role, ToolCall};
use serde::{Deserialize, Serialize};

/// A single conversation history entry.
///
/// The inner `message` is the wire-level shape sent to providers. The
/// runtime-only fields are stripped from the wire but persisted to the
/// session `Storage` for reload (except `sender` and `auto_injected`,
/// which are session-local state that resets on reload).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HistoryEntry {
    /// Which agent produced this assistant message. Empty = the conversation's
    /// primary agent. Non-empty = a guest agent pulled in via an @ mention
    /// or guest turn. Persisted so reloads can reconstruct multi-agent state.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,

    /// The sender identity (runtime-only, never serialized).
    ///
    /// Convention: empty = local/owner, `"tg:12345"` = Telegram user.
    #[serde(skip)]
    pub sender: String,

    /// Whether this entry was auto-injected by the runtime (runtime-only).
    /// Auto-injected entries are stripped before each new run and never
    /// persisted as session steps (see `Conversation::append_messages`).
    #[serde(skip)]
    pub auto_injected: bool,

    /// The wire-level message sent to providers.
    pub message: Message,
}

impl HistoryEntry {
    /// Create a new system entry.
    pub fn system(content: impl Into<String>) -> Self {
        Self::from_message(Message::system(content))
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

    /// Create a new assistant entry.
    ///
    /// Preserves the `content: null` vs empty-string discrimination from the
    /// old `convert::to_ct_message`:
    /// - assistant + non-empty `tool_calls` + empty content → `"content": null`
    /// - assistant + empty `tool_calls` + empty content → `"content": ""`
    /// - anything else → `"content": "<the text>"`
    ///
    /// Rationale: OpenAI accepts `{"role":"assistant","content":null}` without
    /// tool calls; stricter OpenAI-compatible providers (deepseek et al.)
    /// reject it with HTTP 400 "content or tool_calls must be set". Use
    /// `Null` only for the assistant-with-tool-calls-no-text case.
    pub fn assistant(
        content: impl Into<String>,
        reasoning: Option<String>,
        tool_calls: Option<&[ToolCall]>,
    ) -> Self {
        let content: String = content.into();
        let has_tool_calls = tool_calls.is_some_and(|tcs| !tcs.is_empty());
        let message_content = if content.is_empty() && has_tool_calls {
            Some(serde_json::Value::Null)
        } else {
            Some(serde_json::Value::String(content))
        };
        Self::from_message(Message {
            role: Role::Assistant,
            content: message_content,
            tool_calls: tool_calls.map(|tcs| tcs.to_vec()),
            tool_call_id: None,
            name: None,
            reasoning_content: reasoning.filter(|s| !s.is_empty()),
            extra: Default::default(),
        })
    }

    /// Create a new tool-result entry.
    pub fn tool(
        content: impl Into<String>,
        call_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self::from_message(Message::tool(call_id, name, content))
    }

    /// Wrap an existing `crabllm_core::Message` without any runtime-only
    /// fields.
    ///
    /// **Caveat**: this is a raw wrapper. Callers are responsible for the
    /// shape of `message.content` — in particular, the `content: null` vs
    /// empty-string discrimination that `HistoryEntry::assistant` enforces.
    /// Use this only to wrap messages received from the wire (e.g. a
    /// provider response built via `MessageBuilder::build`), where the
    /// content shape is already correct.
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
    pub fn role(&self) -> &Role {
        &self.message.role
    }

    /// The text content of the message, or `""` if absent / empty / non-string.
    ///
    /// Delegates to `Message::content_str`, which collapses absent, null,
    /// empty-string, and multimodal-array content to `None` — this accessor
    /// turns that into `""` for ergonomic call sites that just want the text.
    pub fn text(&self) -> &str {
        self.message.content_str().unwrap_or("")
    }

    /// The reasoning content, or empty if absent.
    pub fn reasoning(&self) -> &str {
        self.message.reasoning_content.as_deref().unwrap_or("")
    }

    /// The tool calls on this entry, or an empty slice if absent.
    pub fn tool_calls(&self) -> &[ToolCall] {
        self.message.tool_calls.as_deref().unwrap_or(&[])
    }

    /// The tool call ID on this (tool) entry, or empty if absent.
    pub fn tool_call_id(&self) -> &str {
        self.message.tool_call_id.as_deref().unwrap_or("")
    }

    /// Estimate the number of tokens in this entry (~4 chars per token).
    pub fn estimate_tokens(&self) -> usize {
        let chars = self.text().len()
            + self.reasoning().len()
            + self.tool_call_id().len()
            + self
                .tool_calls()
                .iter()
                .map(|tc| tc.function.name.len() + tc.function.arguments.len())
                .sum::<usize>();
        (chars / 4).max(1)
    }

    /// Project to a `crabllm_core::Message` for sending to a provider.
    ///
    /// If this is a guest assistant message (`agent` non-empty and role is
    /// Assistant), wraps the content in `<from agent="...">` tags so other
    /// agents can distinguish speakers in multi-agent conversations.
    /// Otherwise returns a clone of the inner message.
    pub fn to_wire_message(&self) -> Message {
        if self.message.role != Role::Assistant || self.agent.is_empty() {
            return self.message.clone();
        }
        let tagged = format!("<from agent=\"{}\">\n{}\n</from>", self.agent, self.text());
        Message {
            role: Role::Assistant,
            content: Some(serde_json::Value::String(tagged)),
            tool_calls: self.message.tool_calls.clone(),
            tool_call_id: self.message.tool_call_id.clone(),
            name: self.message.name.clone(),
            reasoning_content: self.message.reasoning_content.clone(),
            extra: self.message.extra.clone(),
        }
    }
}

/// Estimate total tokens across a slice of entries.
pub fn estimate_tokens(entries: &[HistoryEntry]) -> usize {
    entries.iter().map(|e| e.estimate_tokens()).sum()
}

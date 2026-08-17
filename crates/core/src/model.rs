//! Unified LLM interface types and the `Model<P>` wrapper.
//!
//! Thin re-export layer over `crabllm_core` for the core wire types
//! (`Message`, `Tool`, `ToolCall`, `Usage`, …) plus crabtalk's own
//! `HistoryEntry` wrapper. `Model<P>` is the single seam between crabtalk
//! and any `crabllm_core::Provider`.

pub use crabllm_core::{
    FinishReason, FunctionCall, FunctionDef, Role, Tool, ToolCall, ToolChoice, ToolType, Usage,
    anthropic,
    anthropic::{Content, ContentBlock, Message, ToolResultContent},
    codec::MessageBuilder,
};

use anyhow::Result;
use async_stream::try_stream;
use crabllm_core::Provider;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    ///
    /// Anthropic has no system message role — these are held in history for
    /// the runtime's own bookkeeping and hoisted into `Request.system`.
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
    /// agents can distinguish speakers in multi-agent conversations.
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

/// A wrapper around a `crabllm_core::Provider` that provides a core-typed view.
pub struct Model<P: Provider + 'static> {
    inner: Arc<P>,
}

impl<P: Provider + 'static> Model<P> {
    /// Wrap a provider in a `Model`.
    pub fn new(provider: P) -> Self {
        Self {
            inner: Arc::new(provider),
        }
    }

    /// Wrap an existing `Arc<P>` without re-allocating.
    pub fn from_arc(provider: Arc<P>) -> Self {
        Self { inner: provider }
    }

    /// Send a non-streaming Anthropic messages request.
    pub async fn send(&self, request: anthropic::Request) -> Result<anthropic::Response> {
        let model = request.model.clone();
        self.inner.anthropic_messages(&request).await.map_err(|e| {
            tracing::warn!(model = %model, op = "send", error = %e, "provider request failed");
            anyhow::anyhow!(e.message())
        })
    }

    /// Stream an Anthropic messages response.
    pub fn stream(
        &self,
        request: anthropic::Request,
    ) -> impl Stream<Item = Result<anthropic::StreamEvent>> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        let mut req = request;
        req.stream = Some(true);
        let model = req.model.clone();
        try_stream! {
            let mut stream = inner
                .anthropic_messages_stream(&req)
                .await
                .map_err(|e| {
                    tracing::warn!(model = %model, op = "stream open", error = %e, "provider request failed");
                    anyhow::anyhow!(e.message())
                })?;
            while let Some(chunk) = stream.next().await {
                yield chunk.map_err(|e| {
                    tracing::warn!(model = %model, op = "stream chunk", error = %e, "provider stream failed");
                    anyhow::anyhow!(e.message())
                })?;
            }
        }
    }
}

impl<P: Provider + 'static> Clone for Model<P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<P: Provider + 'static> std::fmt::Debug for Model<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model").finish()
    }
}

//! Unified LLM interface types and the `Model<P>` wrapper.
//!
//! Thin re-export layer over `crabllm_core` for the core wire types
//! (`Message`, `Tool`, `ToolCall`, `Usage`, …) plus crabtalk's own
//! `HistoryEntry` wrapper and streaming `MessageBuilder`. `Model<P>` is the
//! single seam between crabtalk and any `crabllm_core::Provider`.

pub use crabllm_core::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, CompletionTokensDetails,
    FinishReason, FunctionCall, FunctionDef, Message, Role, Tool, ToolCall, ToolCallDelta,
    ToolChoice, ToolType, Usage,
};

use anyhow::Result;
use async_stream::try_stream;
use crabllm_core::{ApiError, Provider};
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

// ── HistoryEntry ────────────────────────────────────────────────────

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

    /// Create a new assistant entry from content blocks.
    pub fn assistant(
        content: impl Into<String>,
        reasoning: Option<String>,
        tool_calls: Option<&[ToolCall]>,
    ) -> Self {
        use crabllm_core::ContentBlock;
        let mut blocks = Vec::new();
        if let Some(r) = reasoning.filter(|s| !s.is_empty()) {
            blocks.push(ContentBlock::Thinking {
                thinking: r,
                signature: None,
            });
        }
        let text: String = content.into();
        if !text.is_empty() || tool_calls.is_none_or(|tcs| tcs.is_empty()) {
            blocks.push(ContentBlock::Text { text });
        }
        if let Some(tcs) = tool_calls {
            for tc in tcs {
                let input = crabllm_core::json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                blocks.push(ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input,
                });
            }
        }
        Self::from_message(Message {
            role: Role::Assistant,
            content: blocks,
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

    /// Wrap an existing `crabllm_core::Message`.
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
    pub fn text(&self) -> &str {
        self.message.content_str().unwrap_or("")
    }

    /// The reasoning/thinking content, or empty if absent.
    pub fn reasoning(&self) -> &str {
        self.message.thinking().unwrap_or("")
    }

    /// The tool calls on this entry as ToolCall structs.
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.message
            .content
            .iter()
            .filter_map(|b| {
                if let crabllm_core::ContentBlock::ToolUse { id, name, input } = b {
                    Some(ToolCall {
                        index: None,
                        id: id.clone(),
                        kind: crabllm_core::ToolType::Function,
                        function: crabllm_core::FunctionCall {
                            name: name.clone(),
                            arguments: crabllm_core::json::to_string(input).unwrap_or_default(),
                        },
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// The tool_use_id from the first ToolResult block, or empty.
    pub fn tool_call_id(&self) -> &str {
        for block in &self.message.content {
            if let crabllm_core::ContentBlock::ToolResult { tool_use_id, .. } = block {
                return tool_use_id.as_str();
            }
        }
        ""
    }

    /// Project to a `crabllm_core::Message` for sending to a provider.
    ///
    /// If this is a guest assistant message (`agent` non-empty and role is
    /// Assistant), wraps the text content in `<from agent="...">` tags so other
    /// agents can distinguish speakers in multi-agent conversations.
    pub fn to_wire_message(&self) -> Message {
        if self.message.role != Role::Assistant || self.agent.is_empty() {
            return self.message.clone();
        }
        let mut blocks = self.message.content.clone();
        for block in &mut blocks {
            if let crabllm_core::ContentBlock::Text { text } = block {
                *text = format!("<from agent=\"{}\">\n{}\n</from>", self.agent, text);
            }
        }
        Message {
            role: Role::Assistant,
            content: blocks,
        }
    }
}

// ── MessageBuilder ──────────────────────────────────────────────────

fn empty_tool_call() -> ToolCall {
    ToolCall {
        index: None,
        id: String::new(),
        kind: ToolType::Function,
        function: FunctionCall::default(),
    }
}

/// Accumulating builder for streaming assistant messages.
pub struct MessageBuilder {
    role: Role,
    content: String,
    reasoning: String,
    calls: BTreeMap<u32, ToolCall>,
}

impl MessageBuilder {
    /// Create a new builder for the given role (typically `Role::Assistant`).
    pub fn new(role: Role) -> Self {
        Self {
            role,
            content: String::new(),
            reasoning: String::new(),
            calls: BTreeMap::new(),
        }
    }

    /// Accept one streaming chunk.
    ///
    /// Returns `true` if this chunk contributed visible text content.
    pub fn accept(&mut self, chunk: &ChatCompletionChunk) -> bool {
        let Some(choice) = chunk.choices.first() else {
            return false;
        };
        let delta = &choice.delta;

        let mut has_content = false;
        if let Some(text) = delta.content.as_deref()
            && !text.is_empty()
        {
            self.content.push_str(text);
            has_content = true;
        }
        if let Some(reason) = delta.reasoning_content.as_deref()
            && !reason.is_empty()
        {
            self.reasoning.push_str(reason);
        }
        if let Some(calls) = delta.tool_calls.as_deref() {
            for call in calls {
                self.merge_tool_call(call);
            }
        }
        has_content
    }

    fn merge_tool_call(&mut self, delta: &ToolCallDelta) {
        let entry = self
            .calls
            .entry(delta.index)
            .or_insert_with(empty_tool_call);
        entry.index = Some(delta.index);
        if let Some(id) = &delta.id
            && !id.is_empty()
        {
            entry.id = id.clone();
        }
        if let Some(kind) = delta.kind {
            entry.kind = kind;
        }
        if let Some(function) = &delta.function {
            if let Some(name) = &function.name
                && !name.is_empty()
            {
                entry.function.name = name.clone();
            }
            if let Some(args) = &function.arguments {
                entry.function.arguments.push_str(args);
            }
        }
    }

    /// Snapshot of tool calls accumulated so far.
    pub fn peek_tool_calls(&self) -> Vec<ToolCall> {
        self.calls
            .values()
            .filter(|c| !c.function.name.is_empty())
            .cloned()
            .collect()
    }

    /// Finalize the builder into a `crabllm_core::Message`.
    pub fn build(self) -> Message {
        use crabllm_core::ContentBlock;

        let mut blocks = Vec::new();

        if !self.reasoning.is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking: self.reasoning,
                signature: None,
            });
        }

        let tool_calls: Vec<ToolCall> = self
            .calls
            .into_values()
            .filter(|c| !c.id.is_empty() && !c.function.name.is_empty())
            .collect();
        let has_tool_calls = !tool_calls.is_empty();

        if !self.content.is_empty() || !has_tool_calls {
            blocks.push(ContentBlock::Text { text: self.content });
        }

        for tc in tool_calls {
            let input = crabllm_core::json::from_str(&tc.function.arguments)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            blocks.push(ContentBlock::ToolUse {
                id: tc.id,
                name: tc.function.name,
                input,
            });
        }

        Message {
            role: self.role,
            content: blocks,
        }
    }
}

// ── Model<P> ────────────────────────────────────────────────────────

/// A wcore-typed view over a `crabllm_core::Provider`.
///
/// Holds an `Arc<P>` so cloning is cheap. The `'static` bound on `P`
/// flows from the streaming path.
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

    /// Send a non-streaming chat completion request.
    pub async fn send_ct(&self, request: ChatCompletionRequest) -> Result<ChatCompletionResponse> {
        let mut req = request;
        req.stream = Some(false);
        let model_label = req.model.clone();
        self.inner
            .chat_completion(&req)
            .await
            .map_err(|e| format_provider_error(&model_label, "send", e))
    }

    /// Stream a chat completion response.
    pub fn stream_ct(
        &self,
        request: ChatCompletionRequest,
    ) -> impl Stream<Item = Result<ChatCompletionChunk>> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        let mut req = request;
        req.stream = Some(true);
        let model_label = req.model.clone();
        try_stream! {
            let mut stream = inner
                .chat_completion_stream(&req)
                .await
                .map_err(|e| format_provider_error(&model_label, "stream open", e))?;
            while let Some(chunk) = stream.next().await {
                yield chunk
                    .map_err(|e| format_provider_error(&model_label, "stream chunk", e))?;
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

fn format_provider_error(model: &str, op: &str, e: crabllm_core::Error) -> anyhow::Error {
    match e {
        crabllm_core::Error::Provider { status, body } => {
            let msg = serde_json::from_str::<ApiError>(&body)
                .map(|api_err| api_err.error.message)
                .unwrap_or_else(|_| truncate(&body, 200));
            anyhow::anyhow!("model {op} failed for '{model}' (HTTP {status}): {msg}")
        }
        other => anyhow::anyhow!("model {op} failed for '{model}': {other}"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

// ── Context limits ──────────────────────────────────────────────────

/// Returns the default context limit (in tokens) for a known model ID.
///
/// Uses prefix matching against known model families. Unknown models
/// return 8192 as a conservative default.
pub fn default_context_limit(model_id: &str) -> usize {
    if model_id.starts_with("claude-") {
        return 200_000;
    }
    if model_id.starts_with("gpt-4o") || model_id.starts_with("gpt-4-turbo") {
        return 128_000;
    }
    if model_id.starts_with("gpt-4") {
        return 8_192;
    }
    if model_id.starts_with("gpt-3.5") {
        return 16_385;
    }
    if model_id.starts_with("o1") || model_id.starts_with("o3") || model_id.starts_with("o4") {
        return 200_000;
    }
    if model_id.starts_with("grok-") {
        return 131_072;
    }
    if model_id.starts_with("qwen-") || model_id.starts_with("qwq-") {
        return 32_768;
    }
    if model_id.starts_with("kimi-") || model_id.starts_with("moonshot-") {
        return 128_000;
    }
    8_192
}

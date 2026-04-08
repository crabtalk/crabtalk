//! `MessageBuilder` — accumulates streaming deltas into a
//! `crabllm_core::Message`.
//!
//! The streaming loop in `Agent::run_stream` feeds each `ChatCompletionChunk`
//! through the builder. Content and reasoning text accumulate as `String`s;
//! tool-call fragments accumulate into a `BTreeMap<u32, ToolCall>` keyed on
//! the delta's `index` (tool calls can arrive out of order and fragmented).
//!
//! Merge rules (pinned by `crates/core/tests/history_pinning.rs`):
//! - `delta.id: Some(..)` overwrites the accumulated id.
//! - `delta.function.name: Some(..)` overwrites the accumulated name.
//! - `delta.function.arguments` always appends to accumulated arguments.
//! - `delta.kind: Some(..)` overwrites the accumulated kind.
//!
//! At `build()` time, content becomes `None` only for the assistant-with-
//! tool-calls-no-text case (matches `HistoryEntry::assistant`). Everything
//! else gets `Some(Value::String(acc))`, even when empty.

use crabllm_core::{
    ChatCompletionChunk, FunctionCall, Message, Role, ToolCall, ToolCallDelta, ToolType,
};
use std::collections::BTreeMap;

/// Build an empty `crabllm_core::ToolCall` with default `ToolType::Function`
/// as the seed for merging streaming deltas.
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
    /// Returns `true` if this chunk contributed visible text content (used by
    /// the agent loop to gate text-segment bracket events).
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

    /// Merge one `ToolCallDelta` fragment into the accumulating tool call
    /// at its index. See module docs for the overwrite/append rules.
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

    /// Snapshot of tool calls accumulated so far, for early-notification UI
    /// events. Only returns calls whose function name has been seen (args may
    /// still be partial). Clones to avoid borrowing `self`.
    pub fn peek_tool_calls(&self) -> Vec<ToolCall> {
        self.calls
            .values()
            .filter(|c| !c.function.name.is_empty())
            .cloned()
            .collect()
    }

    /// Finalize the builder into a `crabllm_core::Message`.
    ///
    /// Preserves the `content: null` discrimination from `HistoryEntry::
    /// assistant`: assistant + non-empty tool calls + empty content →
    /// `Some(Value::Null)` (serializes as `"content": null`). All other
    /// cases get `Some(Value::String(acc))`, even when empty.
    pub fn build(self) -> Message {
        let tool_calls: Vec<ToolCall> = self.calls.into_values().collect();
        let has_tool_calls = !tool_calls.is_empty();
        let content = if self.content.is_empty() && has_tool_calls && self.role == Role::Assistant {
            Some(serde_json::Value::Null)
        } else {
            Some(serde_json::Value::String(self.content))
        };
        let reasoning_content = if self.reasoning.is_empty() {
            None
        } else {
            Some(self.reasoning)
        };
        Message {
            role: self.role,
            content,
            tool_calls: if has_tool_calls {
                Some(tool_calls)
            } else {
                None
            },
            tool_call_id: None,
            name: None,
            reasoning_content,
            extra: Default::default(),
        }
    }
}

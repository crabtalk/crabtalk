//! `TestProvider` — scripted implementation of `crabllm_core::Provider`
//! for use in unit tests and benchmarks.
//!
//! Each constructor takes a fixed sequence of responses or chunk batches
//! that the provider pops on every call. Speaks crabllm-core wire types
//! so tests exercise the real `Model<P>::send` / `stream` conversion path
//! end-to-end.
//!
//! Errors out with `Error::Internal` when the script runs dry, which the
//! agent loop surfaces as an `AgentStopReason::Error` or a regular stream
//! error depending on which path was called.
//!
//! Also exports a handful of fixture constructors (`text_chunk`,
//! `text_response`, `tool_chunks`, etc.) that both `tests/` and
//! `benches/` use to avoid duplicating the same `ChatCompletionChunk {
//! .. }` struct literals across three files.

use crabllm_core::{
    BoxStream, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, Choice,
    ChunkChoice, Delta, Error, FinishReason, FunctionCallDelta, Message, Provider, Role, ToolCall,
    ToolCallDelta, ToolType,
};
use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::{collections::VecDeque, sync::Arc};

/// A mock provider that returns scripted responses in order.
///
/// Thread-safe via `Arc<Mutex<_>>` and `Clone` (cheap — clones share the
/// same underlying script). The provider trait requires `Send + Sync`, both
/// are satisfied.
#[derive(Clone, Default, Debug)]
pub struct TestProvider {
    responses: Arc<Mutex<VecDeque<ChatCompletionResponse>>>,
    chunks: Arc<Mutex<VecDeque<Vec<ChatCompletionChunk>>>>,
}

impl TestProvider {
    /// Create a new test provider with scripted `chat_completion` responses.
    pub fn new(responses: Vec<ChatCompletionResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            chunks: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Create a new test provider with scripted `chat_completion_stream`
    /// chunk batches. Each batch is yielded in full by a single stream call.
    pub fn with_chunks(chunks: Vec<Vec<ChatCompletionChunk>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            chunks: Arc::new(Mutex::new(chunks.into())),
        }
    }

    /// Create a test provider with both chat_completion responses and
    /// chat_completion_stream chunk batches scripted.
    pub fn with_both(
        responses: Vec<ChatCompletionResponse>,
        chunks: Vec<Vec<ChatCompletionChunk>>,
    ) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            chunks: Arc::new(Mutex::new(chunks.into())),
        }
    }
}

impl Provider for TestProvider {
    async fn chat_completion(
        &self,
        _request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, Error> {
        let mut responses = self.responses.lock();
        responses.pop_front().ok_or_else(|| {
            Error::Internal("TestProvider: no more scripted responses for chat_completion".into())
        })
    }

    async fn chat_completion_stream(
        &self,
        _request: &ChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<ChatCompletionChunk, Error>>, Error> {
        let batch = {
            let mut all = self.chunks.lock();
            all.pop_front()
        };
        match batch {
            Some(chunks) => {
                let stream = async_stream::stream! {
                    for chunk in chunks {
                        yield Ok(chunk);
                    }
                };
                Ok(Box::pin(stream))
            }
            None => Err(Error::Internal(
                "TestProvider: no more scripted chunks for chat_completion_stream".into(),
            )),
        }
    }
}

// ── Fixture constructors ──
//
// Shared across `crates/core/tests/` and `crates/bench/benches/`. All
// lean on the `Default` derives on crabllm-core chat types so only the
// fields the test cares about need to be named.

/// A non-streaming chat response carrying `content` as assistant text.
pub fn text_response(content: &str) -> ChatCompletionResponse {
    ChatCompletionResponse {
        choices: vec![Choice {
            index: 0,
            message: Message::assistant(content),
            finish_reason: Some(FinishReason::Stop),
            logprobs: None,
        }],
        ..Default::default()
    }
}

/// A non-streaming chat response carrying one or more tool calls. Uses
/// `content: Null` to match the OpenAI wire convention for tool-call-only
/// assistant messages (where text content is absent rather than empty).
pub fn tool_response(calls: Vec<ToolCall>) -> ChatCompletionResponse {
    ChatCompletionResponse {
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: Some(Value::Null),
                tool_calls: Some(calls),
                tool_call_id: None,
                name: None,
                reasoning_content: None,
                extra: Map::new(),
            },
            finish_reason: Some(FinishReason::ToolCalls),
            logprobs: None,
        }],
        ..Default::default()
    }
}

/// A streaming chunk carrying only a content delta.
pub fn text_chunk(content: &str) -> ChatCompletionChunk {
    ChatCompletionChunk {
        choices: vec![ChunkChoice {
            delta: Delta {
                content: Some(content.into()),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A streaming chunk carrying only a reasoning-content delta.
pub fn thinking_chunk(content: &str) -> ChatCompletionChunk {
    ChatCompletionChunk {
        choices: vec![ChunkChoice {
            delta: Delta {
                reasoning_content: Some(content.into()),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A streaming chunk carrying both content and reasoning in the same delta.
pub fn mixed_chunk(content: &str, reasoning: &str) -> ChatCompletionChunk {
    ChatCompletionChunk {
        choices: vec![ChunkChoice {
            delta: Delta {
                content: Some(content.into()),
                reasoning_content: Some(reasoning.into()),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A terminating stream chunk with the given finish reason and no delta content.
pub fn finish_chunk(reason: FinishReason) -> ChatCompletionChunk {
    ChatCompletionChunk {
        choices: vec![ChunkChoice {
            finish_reason: Some(reason),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Convert a non-streaming `ToolCall` into a streaming `ToolCallDelta`
/// carrying the full name + args in a single delta. Real LLM streams split
/// these across many deltas, but the agent's `MessageBuilder::accept`
/// merges any valid delta sequence — a single-delta emission is the
/// simplest test fixture shape.
pub fn tool_call_delta(tc: &ToolCall) -> ToolCallDelta {
    ToolCallDelta {
        index: tc.index.unwrap_or(0),
        id: Some(tc.id.clone()),
        kind: Some(ToolType::Function),
        function: Some(FunctionCallDelta {
            name: Some(tc.function.name.clone()),
            arguments: Some(tc.function.arguments.clone()),
        }),
    }
}

/// A two-chunk sequence: one chunk carrying all tool-call deltas, followed
/// by a `ToolCalls` finish chunk. The shape streaming agent tests expect
/// from a model that decided to call tools this turn.
pub fn tool_chunks(calls: Vec<ToolCall>) -> Vec<ChatCompletionChunk> {
    let deltas: Vec<ToolCallDelta> = calls.iter().map(tool_call_delta).collect();
    vec![
        ChatCompletionChunk {
            choices: vec![ChunkChoice {
                delta: Delta {
                    tool_calls: Some(deltas),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        },
        finish_chunk(FinishReason::ToolCalls),
    ]
}

/// Split `text` into per-character content chunks followed by a `Stop`
/// finish chunk. Used by streaming agent/runtime tests that want to
/// verify chunk-by-chunk delta accumulation.
pub fn text_chunks(text: &str) -> Vec<ChatCompletionChunk> {
    let mut chunks: Vec<ChatCompletionChunk> =
        text.chars().map(|c| text_chunk(&c.to_string())).collect();
    chunks.push(finish_chunk(FinishReason::Stop));
    chunks
}

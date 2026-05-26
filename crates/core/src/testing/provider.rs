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
    AnthropicContentBlock, AnthropicRequest, AnthropicResponse, AnthropicStreamEvent,
    AnthropicUsage, BlockDelta, BoxStream, ChatCompletionChunk, ChatCompletionRequest,
    ChatCompletionResponse, Choice, ChunkChoice, Delta, Error, FinishReason, FunctionCallDelta,
    GeminiRequest, GeminiResponse, Message, MessageDeltaPayload, Provider, Role, ToolCall,
    ToolCallDelta, ToolType,
};
use futures_util::StreamExt;
use parking_lot::Mutex;
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

    async fn anthropic_messages(
        &self,
        request: &AnthropicRequest,
    ) -> Result<AnthropicResponse, Error> {
        let ir_req = crabllm_core::ir::Request::from(request.clone());
        let chat_req = ChatCompletionRequest::from(&ir_req);
        let resp = self.chat_completion(&chat_req).await?;
        let ir_resp = crabllm_core::ir::Response::from(resp);
        Ok(AnthropicResponse::from(&ir_resp))
    }

    async fn anthropic_messages_stream(
        &self,
        request: &AnthropicRequest,
    ) -> Result<BoxStream<'static, Result<AnthropicStreamEvent, Error>>, Error> {
        let ir_req = crabllm_core::ir::Request::from(request.clone());
        let chat_req = ChatCompletionRequest::from(&ir_req);
        let mut chunks = self.chat_completion_stream(&chat_req).await?;
        let stream = async_stream::stream! {
            let mut block_index = 0u32;
            let mut _tool_idx = 0u32;
            while let Some(result) = chunks.next().await {
                let chunk = match result {
                    Ok(c) => c,
                    Err(e) => { yield Err(e); return; }
                };
                let Some(choice) = chunk.choices.first() else { continue };
                let delta = &choice.delta;
                if let Some(text) = &delta.content {
                    yield Ok(AnthropicStreamEvent::ContentBlockDelta {
                        index: block_index,
                        delta: BlockDelta::Text { text: text.clone() },
                    });
                }
                if let Some(reason) = &delta.reasoning_content {
                    yield Ok(AnthropicStreamEvent::ContentBlockDelta {
                        index: block_index,
                        delta: BlockDelta::Thinking { thinking: reason.clone() },
                    });
                }
                if let Some(calls) = &delta.tool_calls {
                    for tc in calls {
                        if tc.id.is_some() {
                            block_index += 1;
                            _tool_idx = tc.index;
                            yield Ok(AnthropicStreamEvent::ContentBlockStart {
                                index: block_index,
                                content_block: AnthropicContentBlock::ToolUse {
                                    id: tc.id.clone().unwrap_or_default(),
                                    name: tc.function.as_ref().and_then(|f| f.name.clone()).unwrap_or_default(),
                                    input: serde_json::json!({}),
                                    cache_control: None,
                                },
                            });
                        }
                        if let Some(func) = &tc.function {
                            if let Some(args) = &func.arguments {
                                yield Ok(AnthropicStreamEvent::ContentBlockDelta {
                                    index: block_index,
                                    delta: BlockDelta::InputJson { partial_json: args.clone() },
                                });
                            }
                        }
                    }
                }
                if let Some(reason) = &choice.finish_reason {
                    let stop = match reason {
                        FinishReason::Stop => "end_turn",
                        FinishReason::Length => "max_tokens",
                        FinishReason::ToolCalls => "tool_use",
                        _ => "end_turn",
                    };
                    yield Ok(AnthropicStreamEvent::MessageDelta {
                        delta: MessageDeltaPayload {
                            stop_reason: Some(stop.to_string()),
                            stop_sequence: None,
                        },
                        usage: AnthropicUsage {
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_input_tokens: None,
                            cache_creation_input_tokens: None,
                        },
                    });
                }
            }
        };
        Ok(Box::pin(stream))
    }

    async fn gemini_generate_content_stream(
        &self,
        _model: &str,
        _request: &GeminiRequest,
    ) -> Result<BoxStream<'static, Result<GeminiResponse, Error>>, Error> {
        Err(Error::Internal(
            "TestProvider: gemini streaming not supported".into(),
        ))
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
        usage: Some(Default::default()),
        ..Default::default()
    }
}

/// A non-streaming chat response carrying one or more tool calls. Uses
/// `content: Null` to match the OpenAI wire convention for tool-call-only
/// assistant messages (where text content is absent rather than empty).
pub fn tool_response(calls: Vec<ToolCall>) -> ChatCompletionResponse {
    use crabllm_core::ContentBlock;
    let blocks: Vec<ContentBlock> = calls
        .into_iter()
        .map(|tc| {
            let input = crabllm_core::json::from_str(&tc.function.arguments)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            ContentBlock::ToolUse {
                id: tc.id,
                name: tc.function.name,
                input,
                cache_control: None,
            }
        })
        .collect();
    ChatCompletionResponse {
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: Role::Assistant,
                content: blocks,
            },
            finish_reason: Some(FinishReason::ToolCalls),
            logprobs: None,
        }],
        usage: Some(Default::default()),
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

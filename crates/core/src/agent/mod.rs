//! Immutable agent definition and execution methods.
//!
//! [`Agent`] owns its configuration, model, tool schemas, and an optional
//! [`ToolSender`] for dispatching tool calls to the runtime. Conversation
//! history is passed in externally — the agent itself is stateless.
//! It drives LLM execution through [`Agent::step`], [`Agent::run`], and
//! [`Agent::run_stream`]. `run_stream()` is the canonical step loop —
//! `run()` collects its events and returns the final response.

use crate::model::{
    Choice, CompletionMeta, Delta, Message, MessageBuilder, Model, Request, Response, Role, Tool,
    Usage,
};
use anyhow::Result;
use async_stream::stream;
use event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason};
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tool::{ToolRequest, ToolSender};

pub use builder::AgentBuilder;
pub use compact::COMPACT_SENTINEL;
pub use config::AgentConfig;
pub use parser::parse_agent_md;

mod builder;
mod compact;
pub mod config;
pub mod event;
mod parser;
pub mod tool;

/// An immutable agent definition.
///
/// Generic over `M: Model` — stores the model provider alongside config,
/// tool schemas, and an optional sender for tool dispatch. Conversation
/// history is owned externally and passed into execution methods.
/// Callers drive execution via `step()` (single LLM round), `run()` (loop to
/// completion), or `run_stream()` (yields events as a stream).
pub struct Agent<M: Model> {
    /// Agent configuration (name, prompt, model, limits, tool_choice).
    pub config: AgentConfig,
    /// The model provider for LLM calls.
    model: M,
    /// Tool schemas advertised to the LLM. Set once at build time.
    tools: Vec<Tool>,
    /// Sender for dispatching tool calls to the runtime. None = no tools.
    tool_tx: Option<ToolSender>,
}

impl<M: Model> Agent<M> {
    /// Perform a single LLM round: send request, dispatch tools, return step.
    ///
    /// Composes a [`Request`] from config state (system prompt + history +
    /// tool schemas), calls the stored model, dispatches any tool calls via
    /// the [`ToolSender`] channel, and appends results to history.
    pub async fn step(&self, history: &mut Vec<Message>) -> Result<AgentStep> {
        let model_name = self
            .config
            .model
            .clone()
            .unwrap_or_else(|| self.model.active_model());

        let mut messages = Vec::with_capacity(1 + history.len());
        if !self.config.system_prompt.is_empty() {
            messages.push(Message::system(&self.config.system_prompt));
        }
        messages.extend(history.iter().cloned());

        let mut request = Request::new(model_name)
            .with_messages(messages)
            .with_tool_choice(self.config.tool_choice.clone());
        if !self.tools.is_empty() {
            request = request.with_tools(self.tools.clone());
        }

        let response = self.model.send(&request).await?;
        let tool_calls = response.tool_calls().unwrap_or_default().to_vec();

        if let Some(msg) = response.message() {
            history.push(msg);
        }

        let mut tool_results = Vec::new();
        if !tool_calls.is_empty() {
            for tc in &tool_calls {
                let result = self
                    .dispatch_tool(&tc.function.name, &tc.function.arguments)
                    .await;
                let msg = Message::tool(&result, tc.id.clone());
                history.push(msg.clone());
                tool_results.push(msg);
            }
        }

        Ok(AgentStep {
            response,
            tool_calls,
            tool_results,
        })
    }

    /// Dispatch a single tool call via the tool sender channel.
    ///
    /// Returns the result string. If no sender is configured, returns an error
    /// message without panicking.
    async fn dispatch_tool(&self, name: &str, args: &str) -> String {
        let Some(tx) = &self.tool_tx else {
            return format!("tool '{name}' called but no tool sender configured");
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let req = ToolRequest {
            name: name.to_owned(),
            args: args.to_owned(),
            agent: self.config.name.to_string(),
            reply: reply_tx,
            task_id: None,
        };
        if tx.send(req).is_err() {
            return format!("tool channel closed while calling '{name}'");
        }
        reply_rx
            .await
            .unwrap_or_else(|_| format!("tool '{name}' dropped reply"))
    }

    /// Determine the stop reason for a step with no tool calls.
    fn stop_reason(step: &AgentStep) -> AgentStopReason {
        if step.response.content().is_some() {
            AgentStopReason::TextResponse
        } else {
            AgentStopReason::NoAction
        }
    }

    /// Run the agent loop to completion, returning the final response.
    ///
    /// Wraps [`Agent::run_stream`] — collects all events, sends each through
    /// `events`, and extracts the `Done` response.
    pub async fn run(
        &self,
        history: &mut Vec<Message>,
        events: mpsc::UnboundedSender<AgentEvent>,
    ) -> AgentResponse {
        let mut stream = std::pin::pin!(self.run_stream(history));
        let mut response = None;
        while let Some(event) = stream.next().await {
            if let AgentEvent::Done(ref resp) = event {
                response = Some(resp.clone());
            }
            let _ = events.send(event);
        }

        response.unwrap_or_else(|| AgentResponse {
            final_response: None,
            iterations: 0,
            stop_reason: AgentStopReason::Error("stream ended without Done".into()),
            steps: vec![],
        })
    }

    /// Run the agent loop as a stream of [`AgentEvent`]s.
    ///
    /// Uses the model's streaming API so text deltas are yielded token-by-token.
    /// Tool call responses are dispatched after the stream completes (arguments
    /// arrive incrementally and must be fully accumulated first).
    pub fn run_stream<'a>(
        &'a self,
        history: &'a mut Vec<Message>,
    ) -> impl Stream<Item = AgentEvent> + 'a {
        stream! {
            let mut steps = Vec::new();
            let max = self.config.max_iterations;

            for _ in 0..max {
                // Build the request (same logic as step()).
                let model_name = self
                    .config
                    .model
                    .clone()
                    .unwrap_or_else(|| self.model.active_model());

                let mut messages = Vec::with_capacity(1 + history.len());
                if !self.config.system_prompt.is_empty() {
                    messages.push(Message::system(&self.config.system_prompt));
                }
                messages.extend(history.iter().cloned());

                let mut request = Request::new(model_name)
                    .with_messages(messages)
                    .with_tool_choice(self.config.tool_choice.clone());
                if !self.tools.is_empty() {
                    request = request.with_tools(self.tools.clone());
                }

                // Stream from the model, yielding text deltas as they arrive.
                let mut builder = MessageBuilder::new(Role::Assistant);
                let mut finish_reason = None;
                let mut last_meta = CompletionMeta::default();
                let mut last_usage = None;
                let mut stream_error = None;

                {
                    let mut chunk_stream = std::pin::pin!(self.model.stream(request));
                    while let Some(result) = chunk_stream.next().await {
                        match result {
                            Ok(chunk) => {
                                if let Some(text) = chunk.content() {
                                    yield AgentEvent::TextDelta(text.to_owned());
                                }
                                if let Some(r) = chunk.reason() {
                                    finish_reason = Some(*r);
                                }
                                last_meta = chunk.meta.clone();
                                if chunk.usage.is_some() {
                                    last_usage = chunk.usage.clone();
                                }
                                builder.accept(&chunk);
                            }
                            Err(e) => {
                                stream_error = Some(e.to_string());
                                break;
                            }
                        }
                    }
                }
                if let Some(e) = stream_error {
                    yield AgentEvent::Done(AgentResponse {
                        final_response: None,
                        iterations: steps.len(),
                        stop_reason: AgentStopReason::Error(e),
                        steps,
                    });
                    return;
                }

                // Build the accumulated message and response.
                let msg = builder.build();
                let tool_calls = msg.tool_calls.to_vec();
                let content = if msg.content.is_empty() {
                    None
                } else {
                    Some(msg.content.clone())
                };

                let response = Response {
                    meta: last_meta,
                    choices: vec![Choice {
                        index: 0,
                        delta: Delta {
                            role: Some(Role::Assistant),
                            content: content.clone(),
                            reasoning_content: if msg.reasoning_content.is_empty() {
                                None
                            } else {
                                Some(msg.reasoning_content.clone())
                            },
                            tool_calls: if tool_calls.is_empty() {
                                None
                            } else {
                                Some(tool_calls.clone())
                            },
                        },
                        finish_reason,
                        logprobs: None,
                    }],
                    usage: last_usage.unwrap_or(Usage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                        prompt_cache_hit_tokens: None,
                        prompt_cache_miss_tokens: None,
                        completion_tokens_details: None,
                    }),
                };

                history.push(msg);
                let has_tool_calls = !tool_calls.is_empty();

                // Dispatch tool calls if any.
                let mut tool_results = Vec::new();
                let mut compact_triggered = false;
                if has_tool_calls {
                    yield AgentEvent::ToolCallsStart(tool_calls.clone());
                    for tc in &tool_calls {
                        let result = self
                            .dispatch_tool(&tc.function.name, &tc.function.arguments)
                            .await;
                        if result.starts_with(compact::COMPACT_SENTINEL) {
                            compact_triggered = true;
                        }
                        let msg = Message::tool(&result, tc.id.clone());
                        history.push(msg.clone());
                        tool_results.push(msg);
                        yield AgentEvent::ToolResult {
                            call_id: tc.id.clone(),
                            output: result,
                        };
                    }
                    yield AgentEvent::ToolCallsComplete;
                }

                // Handle compaction: summarize history, store journal, replace.
                if compact_triggered {
                    if let Some(summary) = self.compact(history).await {
                        // Store journal entry via internal tool dispatch.
                        let _ = self.dispatch_tool("__journal__", &summary).await;
                        // Replace history with the summary.
                        *history = vec![Message::user(&summary)];
                        yield AgentEvent::TextDelta(
                            "\n[context compacted]\n".to_owned(),
                        );
                    }
                    // Continue the agent loop with compact context.
                    continue;
                }

                let step = AgentStep {
                    response,
                    tool_calls,
                    tool_results,
                };

                if !has_tool_calls {
                    let stop_reason = Self::stop_reason(&step);
                    steps.push(step);
                    yield AgentEvent::Done(AgentResponse {
                        final_response: content,
                        iterations: steps.len(),
                        stop_reason,
                        steps,
                    });
                    return;
                }

                steps.push(step);
            }

            let final_response = steps.last().and_then(|s| s.response.content().cloned());
            yield AgentEvent::Done(AgentResponse {
                final_response,
                iterations: steps.len(),
                stop_reason: AgentStopReason::MaxIterations,
                steps,
            });
        }
    }
}

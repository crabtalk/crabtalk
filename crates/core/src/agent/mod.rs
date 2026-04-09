//! Immutable agent definition and execution methods.
//!
//! [`Agent`] owns its configuration, model, tool schemas, and an optional
//! [`ToolSender`] for dispatching tool calls to the runtime. Conversation
//! history is passed in externally — the agent itself is stateless.
//! It drives LLM execution through [`Agent::step`], [`Agent::run`], and
//! [`Agent::run_stream`]. `run_stream()` is the canonical step loop —
//! `run()` collects its events and returns the final response.

use crate::model::{HistoryEntry, Model, builder::MessageBuilder};
use anyhow::Result;
use async_stream::stream;
pub use builder::AgentBuilder;
pub use config::AgentConfig;
use crabllm_core::{ChatCompletionRequest, Provider, Role, Tool, ToolCall, ToolChoice, Usage};
use event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason};
use futures_core::Stream;
use futures_util::{StreamExt, future::join_all, stream::FuturesUnordered};
use tokio::sync::{mpsc, oneshot, watch};
pub use tool::{AsTool, ToolDescription, ToolRequest, ToolSender};

mod builder;
mod compact;
pub mod config;
pub mod event;
pub mod tool;

/// A neutral placeholder assistant message returned by `step()` when the
/// provider yields zero choices. Used only as a step record so callers see
/// an empty AgentStep instead of a panic; nothing is appended to history.
fn empty_assistant_message() -> crabllm_core::Message {
    crabllm_core::Message {
        role: Role::Assistant,
        content: Some(serde_json::Value::String(String::new())),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
        extra: Default::default(),
    }
}

/// Extract sender from the last user entry in history.
fn last_sender(history: &[HistoryEntry]) -> String {
    history
        .iter()
        .rev()
        .find(|e| *e.role() == Role::User)
        .map(|e| e.sender.clone())
        .unwrap_or_default()
}

/// Borrow the inner string from a tool-dispatch result regardless of
/// success/error. The LLM wire format (crabllm-core `Message`) has no
/// `is_error` flag, so the agent collapses both arms to a plain string
/// when appending to history. UI clients still get the distinction via
/// `AgentEvent::ToolResult.output`.
fn tool_output_text(result: &Result<String, String>) -> &str {
    match result {
        Ok(s) | Err(s) => s,
    }
}

/// An immutable agent definition.
///
/// Generic over `P: crabllm_core::Provider` — holds a `Model<P>` wrapper
/// alongside config, tool schemas, and an optional sender for tool
/// dispatch. Conversation history is owned externally and passed into
/// execution methods. Callers drive execution via `step()` (single LLM
/// round), `run()` (loop to completion), or `run_stream()` (yields events
/// as a stream).
pub struct Agent<P: Provider + 'static> {
    /// Agent configuration (name, prompt, model, limits, tool_choice).
    pub config: AgentConfig,
    /// The model wrapper for LLM calls.
    model: Model<P>,
    /// Tool schemas advertised to the LLM. Set once at build time.
    tools: Vec<Tool>,
    /// Sender for dispatching tool calls to the runtime. None = no tools.
    tool_tx: Option<ToolSender>,
}

impl<P: Provider + 'static> Clone for Agent<P> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
            tool_tx: self.tool_tx.clone(),
        }
    }
}

impl<P: Provider + 'static> Agent<P> {
    /// Resolve the model name from agent config.
    ///
    /// `config.model` is filled at config load time (defaulting from
    /// `system.crab.model` when an agent doesn't set its own), so this is
    /// always `Some` at runtime. The `unwrap_or_default` here is purely
    /// defensive — a missing model would surface as an empty model name in
    /// the request, which the registry will reject with a clear error.
    fn model_name(&self) -> String {
        self.config.model.clone().unwrap_or_default()
    }

    /// Build a `ChatCompletionRequest` from config state (system prompt +
    /// history + tool schemas).
    ///
    /// If `tool_choice_override` is provided, it takes precedence over the
    /// agent config's `tool_choice`. Projects each `HistoryEntry` through
    /// `to_wire_message()` so guest assistant messages get wrapped in
    /// `<from agent="...">` tags.
    fn build_request(
        &self,
        history: &[HistoryEntry],
        tool_choice_override: Option<&ToolChoice>,
    ) -> ChatCompletionRequest {
        let model_name = self.model_name();

        let mut messages = Vec::with_capacity(1 + history.len());
        if !self.config.system_prompt.is_empty() {
            messages.push(crabllm_core::Message::system(&self.config.system_prompt));
        }
        messages.extend(history.iter().map(|e| e.to_wire_message()));

        let tool_choice = tool_choice_override
            .cloned()
            .unwrap_or_else(|| self.config.tool_choice.clone());

        ChatCompletionRequest {
            model: model_name,
            messages,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: None,
            stop: None,
            tools: if self.tools.is_empty() {
                None
            } else {
                Some(self.tools.clone())
            },
            tool_choice: Some(tool_choice),
            frequency_penalty: None,
            presence_penalty: None,
            seed: None,
            user: None,
            reasoning_effort: self.config.thinking.then(|| "high".to_string()),
            extra: Default::default(),
        }
    }

    /// Perform a single LLM round: send request, dispatch tools, return step.
    ///
    /// Composes a [`ChatCompletionRequest`] from config state (system prompt +
    /// history + tool schemas), calls the stored model, dispatches any tool
    /// calls via the [`ToolSender`] channel, and appends results to history.
    pub async fn step(
        &self,
        history: &mut Vec<HistoryEntry>,
        conversation_id: Option<u64>,
    ) -> Result<AgentStep> {
        let request = self.build_request(history, None);
        let response = self.model.send_ct(request).await?;
        let tool_calls: Vec<ToolCall> = response.tool_calls().to_vec();
        let finish_reason = response.finish_reason().cloned();
        let usage = response.usage.clone().unwrap_or_default();

        // If the provider returned zero choices, there is no message to record
        // — match the old `step()` behavior of not appending anything in that
        // case, instead of bloating history with a synthetic empty assistant
        // entry on flaky providers.
        let Some(message) = response.message().cloned() else {
            return Ok(AgentStep {
                message: empty_assistant_message(),
                usage,
                finish_reason,
                tool_calls,
                tool_results: Vec::new(),
            });
        };

        history.push(HistoryEntry::from_message(message.clone()));

        let mut tool_results = Vec::new();
        if !tool_calls.is_empty() {
            let sender = last_sender(history);
            let outputs = join_all(tool_calls.iter().map(|tc| {
                self.dispatch_tool(
                    &tc.function.name,
                    &tc.function.arguments,
                    &sender,
                    conversation_id,
                )
            }))
            .await;
            for (tc, result) in tool_calls.iter().zip(outputs) {
                let entry = HistoryEntry::tool(
                    tool_output_text(&result),
                    tc.id.clone(),
                    &tc.function.name,
                );
                history.push(entry.clone());
                tool_results.push(entry);
            }
        }

        Ok(AgentStep {
            message,
            usage,
            finish_reason,
            tool_calls,
            tool_results,
        })
    }

    /// Dispatch a single tool call via the tool sender channel.
    ///
    /// Returns `Ok(output)` for normal tool output or `Err(message)` for a
    /// failure. Dispatch-level problems (no sender, channel closed, reply
    /// dropped) map to `Err`; the handler's own `Ok`/`Err` verdict is
    /// forwarded unchanged.
    async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        sender: &str,
        conversation_id: Option<u64>,
    ) -> Result<String, String> {
        let Some(tx) = &self.tool_tx else {
            return Err(format!(
                "tool '{name}' called but no tool sender configured"
            ));
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let req = ToolRequest {
            name: name.to_owned(),
            args: args.to_owned(),
            agent: self.config.name.to_string(),
            reply: reply_tx,
            task_id: None,
            sender: sender.into(),
            conversation_id,
        };
        if tx.send(req).is_err() {
            return Err(format!("tool channel closed while calling '{name}'"));
        }
        reply_rx
            .await
            .unwrap_or_else(|_| Err(format!("tool '{name}' dropped reply")))
    }

    /// Determine the stop reason for a step with no tool calls.
    fn stop_reason(step: &AgentStep) -> AgentStopReason {
        let has_text = step
            .message
            .content
            .as_ref()
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if has_text {
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
        history: &mut Vec<HistoryEntry>,
        events: mpsc::UnboundedSender<AgentEvent>,
        conversation_id: Option<u64>,
        tool_choice: Option<ToolChoice>,
    ) -> AgentResponse {
        let mut stream =
            std::pin::pin!(self.run_stream(history, conversation_id, None, tool_choice));
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
            model: self.model_name(),
        })
    }

    /// Run the agent loop as a stream of [`AgentEvent`]s.
    ///
    /// Uses the model's streaming API so text deltas are yielded token-by-token.
    /// Tool call responses are dispatched after the stream completes (arguments
    /// arrive incrementally and must be fully accumulated first).
    pub fn run_stream<'a>(
        &'a self,
        history: &'a mut Vec<HistoryEntry>,
        conversation_id: Option<u64>,
        mut steer_rx: Option<watch::Receiver<Option<String>>>,
        tool_choice: Option<ToolChoice>,
    ) -> impl Stream<Item = AgentEvent> + 'a {
        stream! {
            let mut steps = Vec::new();
            let max = self.config.max_iterations;
            let model_name = self.model_name();

            for _ in 0..max {
                // Check for pending steering message before the next model call.
                // Scope the borrow so the !Send guard is dropped before yield.
                let steer_content = steer_rx.as_mut().and_then(|rx| {
                    rx.has_changed().ok()?.then(|| rx.borrow_and_update().clone())?
                });
                if let Some(content) = steer_content {
                    let sender = last_sender(history);
                    history.push(HistoryEntry::user_with_sender(&content, &sender));
                    yield AgentEvent::UserSteered { content };
                }

                let request = self.build_request(history, tool_choice.as_ref());

                // Stream from the model, yielding text deltas as they arrive.
                let mut builder = MessageBuilder::new(Role::Assistant);
                let mut finish_reason = None;
                let mut last_usage: Option<Usage> = None;
                let mut stream_error = None;
                let mut tool_begin_emitted = false;

                // Tracks the currently open text/thinking segment so we can
                // bracket deltas with explicit Start/End events. Only one
                // segment is open at a time — type transitions emit the
                // closing event for the previous segment first.
                #[derive(PartialEq)]
                enum OpenSegment { None, Text, Thinking }
                let mut open = OpenSegment::None;

                {
                    let mut chunk_stream = std::pin::pin!(self.model.stream_ct(request));
                    while let Some(result) = chunk_stream.next().await {
                        match result {
                            Ok(chunk) => {
                                // Process text portion. Match existing behavior:
                                // emit TextDelta even when the slice is empty.
                                if let Some(text) = chunk.content() {
                                    if open != OpenSegment::Text {
                                        if open == OpenSegment::Thinking {
                                            yield AgentEvent::ThinkingEnd;
                                        }
                                        yield AgentEvent::TextStart;
                                        open = OpenSegment::Text;
                                    }
                                    yield AgentEvent::TextDelta(text.to_owned());
                                }
                                // Process reasoning portion. Same atomic-flip logic.
                                if let Some(reason) = chunk.reasoning_content() {
                                    if open != OpenSegment::Thinking {
                                        if open == OpenSegment::Text {
                                            yield AgentEvent::TextEnd;
                                        }
                                        yield AgentEvent::ThinkingStart;
                                        open = OpenSegment::Thinking;
                                    }
                                    yield AgentEvent::ThinkingDelta(reason.to_owned());
                                }
                                if let Some(r) = chunk.finish_reason() {
                                    finish_reason = Some(r.clone());
                                }
                                if chunk.usage.is_some() {
                                    last_usage = chunk.usage.clone();
                                }
                                builder.accept(&chunk);
                                // Emit ToolCallsBegin as soon as tool names appear
                                // in the builder, so the CLI can show markers while
                                // args are still streaming. Uses current builder
                                // state, which may already have partial/full args.
                                if !tool_begin_emitted {
                                    let calls = builder.peek_tool_calls();
                                    if !calls.is_empty() {
                                        tool_begin_emitted = true;
                                        yield AgentEvent::ToolCallsBegin(calls);
                                    }
                                }
                            }
                            Err(e) => {
                                stream_error = Some(e.to_string());
                                break;
                            }
                        }
                    }
                    // Close whatever segment is still open at end of stream.
                    match open {
                        OpenSegment::Text => yield AgentEvent::TextEnd,
                        OpenSegment::Thinking => yield AgentEvent::ThinkingEnd,
                        OpenSegment::None => {}
                    }
                }
                if let Some(e) = stream_error {
                    yield AgentEvent::Done(AgentResponse {
                        final_response: None,
                        iterations: steps.len(),
                        stop_reason: AgentStopReason::Error(e),
                        steps,
                        model: model_name.clone(),
                    });
                    return;
                }

                // Build the accumulated message. `MessageBuilder::build`
                // already drops degenerate (id-less or name-less) tool call
                // fragments, so any tool_calls present here are well-formed.
                let message = builder.build();
                let tool_calls: Vec<ToolCall> =
                    message.tool_calls.clone().unwrap_or_default();
                let content = message
                    .content
                    .as_ref()
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_owned());
                let usage = last_usage.unwrap_or_default();
                let has_tool_calls = !tool_calls.is_empty();

                // If the stream produced neither text nor any usable tool
                // call, treat the round as a no-op: do not push the empty
                // assistant message into history (which would persist via
                // `append_messages` and contaminate the next request),
                // yield Done with NoAction, and return. This is the
                // mid-stream-disconnect path — reqwest can end an SSE
                // stream cleanly with `Ok(None)` on a TCP RST, so we
                // can't rely on `stream_error` alone to catch it.
                if content.is_none() && !has_tool_calls {
                    yield AgentEvent::Done(AgentResponse {
                        final_response: None,
                        iterations: steps.len(),
                        stop_reason: AgentStopReason::NoAction,
                        steps,
                        model: model_name.clone(),
                    });
                    return;
                }

                history.push(HistoryEntry::from_message(message.clone()));

                // Dispatch tool calls concurrently.
                //
                // `FuturesUnordered` polls each dispatch future to completion
                // independently so `ToolResult` events fire in completion
                // order (fast tools don't wait on slow siblings in the UI).
                // Outputs are buffered by the original call index so history
                // entries append in call order — providers pair results to
                // calls by position in some encodings, so this ordering is
                // load-bearing.
                let mut tool_results = Vec::new();
                if has_tool_calls {
                    let sender = last_sender(history);
                    yield AgentEvent::ToolCallsStart(tool_calls.clone());

                    let mut pending: FuturesUnordered<_> = tool_calls
                        .iter()
                        .enumerate()
                        .map(|(idx, tc)| {
                            let fut = self.dispatch_tool(
                                &tc.function.name,
                                &tc.function.arguments,
                                &sender,
                                conversation_id,
                            );
                            // `start` is captured inside the async block so
                            // it measures actual polled runtime, not the time
                            // since `FuturesUnordered` was built.
                            async move {
                                let start = std::time::Instant::now();
                                let out = fut.await;
                                (idx, out, start.elapsed().as_millis() as u64)
                            }
                        })
                        .collect();

                    let mut buffered: Vec<Option<Result<String, String>>> =
                        vec![None; tool_calls.len()];
                    while let Some((idx, output, duration_ms)) = pending.next().await {
                        let call_id = tool_calls[idx].id.clone();
                        // Clone into the event; the owned Result lands in
                        // `buffered[idx]` so the drain-loop tail can append
                        // history entries in original call order.
                        yield AgentEvent::ToolResult {
                            call_id,
                            output: output.clone(),
                            duration_ms,
                        };
                        buffered[idx] = Some(output);
                    }

                    for (tc, out) in tool_calls.iter().zip(buffered.into_iter()) {
                        let out = out.expect("FuturesUnordered drained every slot");
                        let entry = HistoryEntry::tool(
                            tool_output_text(&out),
                            tc.id.clone(),
                            &tc.function.name,
                        );
                        history.push(entry.clone());
                        tool_results.push(entry);
                    }

                    yield AgentEvent::ToolCallsComplete;
                }

                // Auto-compaction: check token estimate after each step.
                if let Some(threshold) = self.config.compact_threshold
                    && Self::estimate_tokens(history) > threshold
                {
                    if let Some(summary) = self.compact(history).await {
                        yield AgentEvent::Compact { summary: summary.clone() };
                        *history = vec![HistoryEntry::user(&summary)];
                        yield AgentEvent::TextStart;
                        yield AgentEvent::TextDelta(
                            "\n[context compacted]\n".to_owned(),
                        );
                        yield AgentEvent::TextEnd;
                    }
                    continue;
                }

                let step = AgentStep {
                    message,
                    usage,
                    finish_reason,
                    tool_calls,
                    tool_results,
                };

                if !step.tool_calls.is_empty() {
                    steps.push(step);
                } else {
                    let stop_reason = Self::stop_reason(&step);
                    steps.push(step);
                    yield AgentEvent::Done(AgentResponse {
                        final_response: content,
                        iterations: steps.len(),
                        stop_reason,
                        steps,
                        model: model_name.clone(),
                    });
                    return;
                }
            }

            let final_response = steps
                .last()
                .and_then(|s| s.message.content.as_ref())
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_owned());
            yield AgentEvent::Done(AgentResponse {
                final_response,
                iterations: steps.len(),
                stop_reason: AgentStopReason::MaxIterations,
                steps,
                model: model_name,
            });
        }
    }
}

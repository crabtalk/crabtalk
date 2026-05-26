//! Immutable agent definition and execution methods.
//!
//! [`Agent`] owns its configuration, model, tool schemas, and an optional
//! [`ToolDispatcher`] handle for executing tool calls. Conversation
//! history is passed in externally — the agent itself is stateless.
//! It drives LLM execution through [`Agent::step`], [`Agent::run`], and
//! [`Agent::run_stream`]. `run_stream()` is the canonical step loop —
//! `run()` collects its events and returns the final response.

use crate::model::{HistoryEntry, MessageBuilder, Model};
use anyhow::Result;
use async_stream::stream;
pub use builder::AgentBuilder;
pub use config::AgentConfig;
use crabllm_core::{
    AnthropicContent, AnthropicMessage, AnthropicMessages, AnthropicRequest, AnthropicSystem,
    AnthropicTool, ContentBlock, DEFAULT_MAX_TOKENS, Provider, Role, ThinkingConfig, Tool,
    ToolCall, ToolChoice, Usage,
};
use event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason};
use futures_core::Stream;
use futures_util::{StreamExt, future::join_all, stream::FuturesUnordered};
pub use id::AgentId;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
pub use tool::{AsTool, ToolDispatcher};

mod builder;
mod compact;
pub mod config;
pub mod event;
mod id;
pub mod tool;

fn extract_tool_calls(blocks: &[ContentBlock]) -> Vec<ToolCall> {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse {
                id, name, input, ..
            } => Some(ToolCall {
                index: None,
                id: id.clone(),
                kind: crabllm_core::ToolType::Function,
                function: crabllm_core::FunctionCall {
                    name: name.clone(),
                    arguments: crabllm_core::json::to_string(input).unwrap_or_default(),
                },
            }),
            _ => None,
        })
        .collect()
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
    /// Dispatcher for tool calls. None = no tools.
    dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

impl<P: Provider + 'static> Clone for Agent<P> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
            dispatcher: self.dispatcher.clone(),
        }
    }
}

impl<P: Provider + 'static> Agent<P> {
    /// Append additional tool schemas (e.g. client-provided tools for a
    /// specific conversation). Call on a cloned agent before running.
    pub fn extend_tools(&mut self, tools: Vec<Tool>) {
        self.tools.extend(tools);
    }

    /// Resolve the model name from agent config.
    fn model_name(&self) -> String {
        self.config.model.clone()
    }

    /// Build an `AnthropicRequest` from config state (system prompt +
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
    ) -> AnthropicRequest {
        let model_name = self.model_name();

        let mut messages: Vec<AnthropicMessage> = history
            .iter()
            .map(|e| {
                let msg = e.to_wire_message();
                AnthropicMessage {
                    role: msg.role.as_str().to_string(),
                    content: AnthropicContent::Blocks(msg.content),
                }
            })
            .collect();
        messages.coalesce_tool_results();
        messages.ensure_tool_pairing();

        let system = if self.config.system_prompt.is_empty() {
            None
        } else {
            Some(AnthropicSystem::Text(self.config.system_prompt.clone()))
        };

        let tool_choice = tool_choice_override
            .cloned()
            .unwrap_or_else(|| self.config.tool_choice.clone());

        let is_disabled = tool_choice == ToolChoice::Disabled;

        let tools = if is_disabled || self.tools.is_empty() {
            None
        } else {
            Some(tools_to_anthropic(&self.tools))
        };

        let tool_choice = if is_disabled || self.tools.is_empty() {
            None
        } else {
            Some(tool_choice_to_anthropic(&tool_choice))
        };

        let max_tokens = DEFAULT_MAX_TOKENS;
        let thinking = self.config.thinking.then(|| ThinkingConfig {
            kind: "enabled".to_string(),
            budget_tokens: Some(max_tokens.saturating_sub(1)),
        });

        AnthropicRequest {
            model: model_name,
            messages,
            max_tokens,
            system,
            temperature: None,
            top_p: None,
            stream: None,
            tools,
            tool_choice,
            stop_sequences: None,
            thinking,
        }
    }

    /// Perform a single LLM round: send request, dispatch tools, return step.
    ///
    /// Composes an [`AnthropicRequest`] from config state (system prompt +
    /// history + tool schemas), calls the stored model, dispatches any tool
    /// calls via the [`ToolDispatcher`], and appends results to history.
    pub async fn step(
        &self,
        history: &mut Vec<HistoryEntry>,
        conversation_id: Option<u64>,
    ) -> Result<AgentStep> {
        use crate::model::map_stop_reason;

        let request = self.build_request(history, None);
        let response = self.model.send(request).await?;
        let tool_calls: Vec<ToolCall> = extract_tool_calls(&response.content);
        let finish_reason = map_stop_reason(&response.stop_reason);
        let usage = Usage::from(&response.usage);

        let message = crabllm_core::Message {
            role: Role::Assistant,
            content: response.content,
        };

        let assistant_entry = HistoryEntry::from_message(message.clone());

        let mut tool_results = Vec::new();
        if !tool_calls.is_empty() {
            let sender = last_sender(history);
            let outputs = join_all(tool_calls.iter().map(|tc| {
                self.dispatch_tool(
                    &tc.function.name,
                    &tc.function.arguments,
                    &sender,
                    conversation_id,
                    &tc.id,
                )
            }))
            .await;
            // Commit assistant + tool_results atomically (no `await` between
            // pushes). If this future is cancelled before reaching this block,
            // neither lands in history — the tool_use/tool_result invariant
            // Anthropic requires can never be broken by a partial step.
            history.push(assistant_entry);
            for (tc, result) in tool_calls.iter().zip(outputs) {
                let entry =
                    HistoryEntry::tool(tool_output_text(&result), tc.id.clone(), &tc.function.name);
                history.push(entry.clone());
                tool_results.push(entry);
            }
        } else {
            history.push(assistant_entry);
        }

        Ok(AgentStep {
            message,
            usage,
            finish_reason,
            tool_calls,
            tool_results,
        })
    }

    /// Dispatch a single tool call via the configured [`ToolDispatcher`].
    ///
    /// Returns `Ok(output)` for normal tool output or `Err(message)` for a
    /// failure. If no dispatcher is configured, returns an `Err` describing
    /// the misconfiguration; otherwise the dispatcher's verdict is forwarded
    /// unchanged.
    async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        sender: &str,
        conversation_id: Option<u64>,
        call_id: &str,
    ) -> Result<String, String> {
        let Some(dispatcher) = &self.dispatcher else {
            return Err(format!(
                "tool '{name}' called but no tool dispatcher configured"
            ));
        };
        dispatcher
            .dispatch(
                name,
                args,
                &self.config.name,
                sender,
                conversation_id,
                call_id,
            )
            .await
    }

    /// Determine the stop reason for a step with no tool calls.
    fn stop_reason(step: &AgentStep) -> AgentStopReason {
        if step.message.content_str().is_some() {
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

                let mut builder = MessageBuilder::new(Role::Assistant);
                let mut finish_reason = None;
                let mut last_usage: Option<Usage> = None;
                let mut stream_error = None;
                let mut tool_begin_emitted = false;

                #[derive(PartialEq)]
                enum OpenSegment { None, Text, Thinking }
                let mut open = OpenSegment::None;

                {
                    use crate::model::map_stop_reason_str;
                    use crabllm_core::{AnthropicStreamEvent, BlockDelta};

                    let mut event_stream = std::pin::pin!(self.model.stream(request));
                    while let Some(result) = event_stream.next().await {
                        match result {
                            Ok(ref event) => {
                                match event {
                                    AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                                        match delta {
                                            BlockDelta::Text { text } => {
                                                if open != OpenSegment::Text {
                                                    if open == OpenSegment::Thinking {
                                                        yield AgentEvent::ThinkingEnd;
                                                    }
                                                    yield AgentEvent::TextStart;
                                                    open = OpenSegment::Text;
                                                }
                                                yield AgentEvent::TextDelta(text.clone());
                                            }
                                            BlockDelta::Thinking { thinking } => {
                                                if !thinking.is_empty() {
                                                    if open != OpenSegment::Thinking {
                                                        if open == OpenSegment::Text {
                                                            yield AgentEvent::TextEnd;
                                                        }
                                                        yield AgentEvent::ThinkingStart;
                                                        open = OpenSegment::Thinking;
                                                    }
                                                    yield AgentEvent::ThinkingDelta(thinking.clone());
                                                }
                                            }
                                            BlockDelta::InputJson { .. } => {}
                                        }
                                    }
                                    AnthropicStreamEvent::MessageDelta { delta, usage } => {
                                        finish_reason = delta.stop_reason.as_deref().map(map_stop_reason_str);
                                        last_usage = Some(Usage::from(usage));
                                    }
                                    _ => {}
                                }
                                builder.accept(event);
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
                let tool_calls: Vec<ToolCall> = extract_tool_calls(&message.content);
                let content = message.content_str().map(|s| s.to_owned());
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

                let assistant_entry = HistoryEntry::from_message(message.clone());

                // Dispatch tool calls concurrently.
                //
                // `FuturesUnordered` polls each dispatch future to completion
                // independently so `ToolResult` events fire in completion
                // order (fast tools don't wait on slow siblings in the UI).
                // Outputs are buffered by the original call index so history
                // entries append in call order — providers pair results to
                // calls by position in some encodings, so this ordering is
                // load-bearing.
                //
                // The assistant message is only committed AFTER the dispatch
                // loop drains, so a cancellation during dispatch leaves
                // history untouched — no orphan tool_use without tool_result.
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
                                &tc.id,
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

                    // Atomic commit: push assistant + tool_results with no
                    // `await` between. See comment above the dispatch block.
                    history.push(assistant_entry);
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
                } else {
                    history.push(assistant_entry);
                }

                // Surface real token counts after each LLM call so
                // clients can detect context pressure and decide when to
                // call `compact_conversation`. The daemon does not act on
                // this — policy is the client's.
                if usage.total_tokens() > 0 {
                    yield AgentEvent::ContextUsage { usage: usage.clone() };
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
                .and_then(|s| s.message.content_str())
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

fn tools_to_anthropic(tools: &[Tool]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            input_schema: t
                .function
                .parameters
                .clone()
                .unwrap_or(serde_json::json!({"type": "object"})),
        })
        .collect()
}

fn tool_choice_to_anthropic(tc: &ToolChoice) -> serde_json::Value {
    match tc {
        ToolChoice::Auto => serde_json::json!({"type": "auto"}),
        ToolChoice::Required => serde_json::json!({"type": "any"}),
        ToolChoice::Function { name } => serde_json::json!({"type": "tool", "name": name}),
        ToolChoice::Disabled => serde_json::json!({"type": "none"}),
    }
}

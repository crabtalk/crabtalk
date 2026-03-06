//! Stateful agent execution unit.
//!
//! [`Agent`] owns its configuration, model, message history, tool schemas,
//! and an optional [`ToolSender`] for dispatching tool calls to the runtime.
//! It drives LLM execution through [`Agent::step`], [`Agent::run`], and
//! [`Agent::run_stream`]. `run_stream()` is the canonical step loop —
//! `run()` collects its events and returns the final response.

use crate::model::{Message, Model, Request, Tool};
use anyhow::Result;
use async_stream::stream;
use event::{AgentEvent, AgentResponse, AgentStep, AgentStopReason};
use futures_core::Stream;
use tokio::sync::{mpsc, oneshot};
use tool::{ToolRequest, ToolSender};

pub use builder::AgentBuilder;
pub use config::AgentConfig;
pub use parser::parse_agent_md;

mod builder;
pub mod config;
pub mod event;
mod parser;
pub mod tool;

/// A stateful agent execution unit.
///
/// Generic over `M: Model` — stores the model provider alongside config,
/// conversation history, tool schemas, and an optional sender for tool dispatch.
/// Callers drive execution via `step()` (single LLM round), `run()` (loop to
/// completion), or `run_stream()` (yields events as a stream).
pub struct Agent<M: Model> {
    /// Agent configuration (name, prompt, model, limits, tool_choice).
    pub config: AgentConfig,
    /// The model provider for LLM calls.
    model: M,
    /// Conversation history (user/assistant/tool messages).
    pub(crate) history: Vec<Message>,
    /// Tool schemas advertised to the LLM. Set once at build time.
    tools: Vec<Tool>,
    /// Sender for dispatching tool calls to the runtime. None = no tools.
    tool_tx: Option<ToolSender>,
}

impl<M: Model> Agent<M> {
    /// Push a message into the conversation history.
    pub fn push_message(&mut self, message: Message) {
        self.history.push(message);
    }

    /// Return a reference to the conversation history.
    pub fn messages(&self) -> &[Message] {
        &self.history
    }

    /// Clear the conversation history, keeping configuration intact.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Perform a single LLM round: send request, dispatch tools, return step.
    ///
    /// Composes a [`Request`] from config state (system prompt + history +
    /// tool schemas), calls the stored model, dispatches any tool calls via
    /// the [`ToolSender`] channel, and appends results to history.
    pub async fn step(&mut self) -> Result<AgentStep> {
        let model_name = self
            .config
            .model
            .clone()
            .unwrap_or_else(|| self.model.active_model());

        let mut messages = Vec::with_capacity(1 + self.history.len());
        if !self.config.system_prompt.is_empty() {
            messages.push(Message::system(&self.config.system_prompt));
        }
        messages.extend(self.history.iter().cloned());

        let mut request = Request::new(model_name)
            .with_messages(messages)
            .with_tool_choice(self.config.tool_choice.clone());
        if !self.tools.is_empty() {
            request = request.with_tools(self.tools.clone());
        }

        let response = self.model.send(&request).await?;
        let tool_calls = response.tool_calls().unwrap_or_default().to_vec();

        if let Some(msg) = response.message() {
            self.history.push(msg);
        }

        let mut tool_results = Vec::new();
        if !tool_calls.is_empty() {
            for tc in &tool_calls {
                let result = self
                    .dispatch_tool(&tc.function.name, &tc.function.arguments)
                    .await;
                let msg = Message::tool(&result, tc.id.clone());
                self.history.push(msg.clone());
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
            reply: reply_tx,
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
    pub async fn run(&mut self, events: mpsc::UnboundedSender<AgentEvent>) -> AgentResponse {
        use futures_util::StreamExt;

        let mut stream = std::pin::pin!(self.run_stream());
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
    /// The canonical step loop. Calls [`Agent::step`] up to `max_iterations`
    /// times, yielding events as they are produced. Always finishes with a
    /// `Done` event containing the [`AgentResponse`].
    pub fn run_stream(&mut self) -> impl Stream<Item = AgentEvent> + '_ {
        stream! {
            let mut steps = Vec::new();
            let max = self.config.max_iterations;

            for _ in 0..max {
                match self.step().await {
                    Ok(step) => {
                        let has_tool_calls = !step.tool_calls.is_empty();
                        let text = step.response.content().cloned();

                        if let Some(ref t) = text {
                            yield AgentEvent::TextDelta(t.clone());
                        }

                        if has_tool_calls {
                            yield AgentEvent::ToolCallsStart(step.tool_calls.clone());
                            for (tc, result) in step.tool_calls.iter().zip(&step.tool_results) {
                                yield AgentEvent::ToolResult {
                                    call_id: tc.id.clone(),
                                    output: result.content.clone(),
                                };
                            }
                            yield AgentEvent::ToolCallsComplete;
                        }

                        if !has_tool_calls {
                            let stop_reason = Self::stop_reason(&step);
                            steps.push(step);
                            yield AgentEvent::Done(AgentResponse {
                                final_response: text,
                                iterations: steps.len(),
                                stop_reason,
                                steps,
                            });
                            return;
                        }

                        steps.push(step);
                    }
                    Err(e) => {
                        yield AgentEvent::Done(AgentResponse {
                            final_response: None,
                            iterations: steps.len(),
                            stop_reason: AgentStopReason::Error(e.to_string()),
                            steps,
                        });
                        return;
                    }
                }
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

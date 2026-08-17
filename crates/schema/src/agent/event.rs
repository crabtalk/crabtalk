//! Agent event types for step-based execution and streaming.

use crate::{EventLine, HistoryEntry, ToolCallTrace};
use crabllm_core::{FinishReason, ToolCall, Usage, anthropic::Message};
use proto::*;

/// A fine-grained event emitted during agent execution.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A text segment is starting; subsequent `TextDelta`s belong to it.
    TextStart,
    /// Text content delta from the model.
    TextDelta(String),
    /// The current text segment has ended.
    TextEnd,
    /// A thinking segment is starting; subsequent `ThinkingDelta`s belong to it.
    ThinkingStart,
    /// Thinking/reasoning content delta from the model.
    ThinkingDelta(String),
    /// The current thinking segment has ended.
    ThinkingEnd,
    /// Early notification: model is generating tool calls (names only, args incomplete).
    ToolCallsBegin(Vec<ToolCall>),
    /// Model is calling tools (with the complete tool calls).
    ToolCallsStart(Vec<ToolCall>),
    /// A single tool completed execution.
    ToolResult {
        /// The tool call ID this result belongs to.
        call_id: String,
        /// Success or error output from the tool.
        output: Result<String, String>,
        /// Wall-clock duration of the tool dispatch in milliseconds.
        duration_ms: u64,
    },
    /// All tools completed, continuing to next iteration.
    ToolCallsComplete,
    /// User steering message injected at turn boundary.
    UserSteered { content: String },
    /// Token usage reported by the model after a completed step.
    ContextUsage { usage: Usage },
    /// Agent finished with final response.
    Done(AgentResponse),
}

impl AgentEvent {
    /// Map one agent loop event to its wire `StreamEvent`. Shared by the
    /// persisted and ephemeral stream paths; client-tool forwarding (which
    /// only the persisted path can route) is handled by the caller.
    pub fn to_stream(self, responding_agent: &str) -> StreamEvent {
        let event = self;
        use stream_event::Event;
        let event = match event {
            AgentEvent::TextStart => Event::TextStart(TextStartEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::TextDelta(text) => Event::Chunk(StreamChunk { content: text }),
            AgentEvent::TextEnd => Event::TextEnd(TextEndEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::ThinkingStart => Event::ThinkingStart(ThinkingStartEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::ThinkingDelta(text) => Event::Thinking(StreamThinking { content: text }),
            AgentEvent::ThinkingEnd => Event::ThinkingEnd(ThinkingEndEvent {
                agent: responding_agent.to_string(),
            }),
            AgentEvent::ToolCallsBegin(calls) => Event::ToolStart(ToolStartEvent {
                calls: calls
                    .into_iter()
                    .map(|c| ToolCallInfo {
                        name: c.function.name.to_string(),
                        arguments: String::new(),
                    })
                    .collect(),
            }),
            AgentEvent::ToolCallsStart(calls) => Event::ToolStart(ToolStartEvent {
                calls: calls
                    .into_iter()
                    .map(|c| ToolCallInfo {
                        name: c.function.name.to_string(),
                        arguments: c.function.arguments,
                    })
                    .collect(),
            }),
            AgentEvent::ToolResult {
                call_id,
                output,
                duration_ms,
            } => {
                let is_error = output.is_err();
                let output = match output {
                    Ok(s) | Err(s) => s,
                };
                Event::ToolResult(ToolResultEvent {
                    call_id: call_id.to_string(),
                    output,
                    duration_ms,
                    is_error,
                })
            }
            AgentEvent::ToolCallsComplete => Event::ToolsComplete(ToolsCompleteEvent {}),
            AgentEvent::ContextUsage { usage } => Event::ContextUsage(ContextUsageEvent {
                usage: Some((&usage).into()),
            }),
            AgentEvent::UserSteered { content } => Event::UserSteered(UserSteeredEvent { content }),
            AgentEvent::Done(resp) => {
                let error = if let crate::AgentStopReason::Error(ref e) = resp.stop_reason {
                    e.clone()
                } else {
                    String::new()
                };
                let usage = Some(resp.usage());
                Event::End(StreamEnd {
                    agent: responding_agent.to_string(),
                    error,
                    model: resp.model,
                    usage,
                })
            }
        };
        StreamEvent { event: Some(event) }
    }
}

/// Data record of one LLM round (one model call + tool dispatch).
#[derive(Debug, Clone)]
pub struct AgentStep {
    /// The assistant message produced by this step.
    pub message: Message,
    /// Token usage reported by the provider (zero if not reported).
    pub usage: Usage,
    /// Why the model stopped generating (if reported).
    pub finish_reason: Option<FinishReason>,
    /// Tool calls made in this step (if any).
    pub tool_calls: Vec<ToolCall>,
    /// Results from tool executions as history entries.
    pub tool_results: Vec<HistoryEntry>,
}

/// Final response from a complete agent run.
#[derive(Debug, Clone)]
pub struct AgentResponse {
    /// All steps taken during execution.
    pub steps: Vec<AgentStep>,
    /// Final text response (if any).
    pub final_response: Option<String>,
    /// Total number of iterations performed.
    pub iterations: usize,
    /// Why the agent stopped.
    pub stop_reason: AgentStopReason,
    /// The requested model name (from config, not the API-echoed value).
    pub model: String,
}

impl AgentResponse {
    /// Shorthand for a pre-run error (no steps, no model involved).
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            steps: vec![],
            final_response: None,
            iterations: 0,
            stop_reason: AgentStopReason::Error(msg.into()),
            model: String::new(),
        }
    }
}

/// Why the agent stopped executing.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStopReason {
    /// Model produced a text response with no tool calls.
    TextResponse,
    /// Maximum iterations reached.
    MaxIterations,
    /// No tool calls and no text response.
    NoAction,
    /// The model hit its output token limit — the response is truncated.
    MaxTokens,
    /// Error during execution.
    Error(String),
}

impl AgentResponse {
    /// Total usage across every step of a run.
    pub fn usage(&self) -> TokenUsage {
        let mut prompt = 0u32;
        let mut completion = 0u32;
        let mut total = 0u32;
        let mut cache_hit = 0u32;
        let mut cache_miss = 0u32;
        let mut reasoning = 0u32;

        for step in &self.steps {
            let u = &step.usage;
            prompt += u.prompt_tokens();
            completion += u.completion_tokens();
            total += u.total_tokens();
            cache_hit += u.cache_read_tokens;
            cache_miss += u.cache_write_tokens;
            reasoning += u.reasoning_tokens;
        }

        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cache_hit_tokens: (cache_hit > 0).then_some(cache_hit),
            cache_miss_tokens: (cache_miss > 0).then_some(cache_miss),
            reasoning_tokens: (reasoning > 0).then_some(reasoning),
        }
    }
}

impl std::fmt::Display for AgentStopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextResponse => write!(f, "text_response"),
            Self::MaxIterations => write!(f, "max_iterations"),
            Self::NoAction => write!(f, "no_action"),
            Self::MaxTokens => write!(f, "max_tokens"),
            Self::Error(msg) => write!(f, "error: {msg}"),
        }
    }
}

impl AgentEvent {
    /// Build a trace entry from this event. `None` for events that carry
    /// no useful trace information.
    pub fn to_event_line(&self) -> Option<EventLine> {
        let ts = chrono::Utc::now().to_rfc3339();
        match self {
            AgentEvent::ToolCallsStart(calls) => Some(EventLine::ToolStart {
                calls: calls
                    .iter()
                    .map(|c| ToolCallTrace {
                        id: c.id.clone(),
                        name: c.function.name.to_string(),
                        arguments: c.function.arguments.clone(),
                    })
                    .collect(),
                ts,
            }),
            AgentEvent::ToolResult {
                call_id,
                duration_ms,
                ..
            } => Some(EventLine::ToolResult {
                call_id: call_id.clone(),
                duration_ms: *duration_ms,
                ts,
            }),
            AgentEvent::Done(resp) => Some(EventLine::Done {
                model: resp.model.clone(),
                iterations: resp.iterations,
                stop_reason: resp.stop_reason.to_string(),
                usage: sum_step_usage(&resp.steps),
                ts,
            }),
            AgentEvent::UserSteered { content } => Some(EventLine::UserSteered {
                content: content.clone(),
                ts,
            }),
            _ => None,
        }
    }
}

/// Sum token usage across all steps.
fn sum_step_usage(steps: &[AgentStep]) -> Usage {
    steps.iter().fold(Usage::default(), |mut acc, step| {
        let u = &step.usage;
        acc.input_tokens += u.input_tokens;
        acc.cache_read_tokens += u.cache_read_tokens;
        acc.cache_write_tokens += u.cache_write_tokens;
        acc.output_tokens += u.output_tokens;
        acc.reasoning_tokens += u.reasoning_tokens;
        for (k, v) in &u.server_tool_calls {
            *acc.server_tool_calls.entry(k.clone()).or_insert(0) += v;
        }
        acc
    })
}

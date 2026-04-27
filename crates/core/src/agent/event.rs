//! Agent event types for step-based execution and streaming.
//!
//! Two-level design:
//! - [`AgentStep`]: data record of one LLM round (response + tool dispatch).
//! - [`AgentEvent`]: fine-grained streaming enum for real-time UI updates.
//! - [`AgentResponse`]: final result after a full agent run.
//! - [`AgentStopReason`]: why the agent stopped.

use crate::model::HistoryEntry;
use crabllm_core::{FinishReason, Message, ToolCall, Usage};

/// A fine-grained event emitted during agent execution.
///
/// Yielded by `Agent::run_stream()` or emitted via `Hook::on_event()`
/// for real-time status reporting to clients.
///
/// Text and thinking deltas are bracketed by explicit
/// `TextStart`/`TextEnd` and `ThinkingStart`/`ThinkingEnd` markers so
/// clients can render coherent segments without inferring boundaries
/// from neighboring events. Only one segment is open at a time —
/// transitions emit the closing event of the previous segment before
/// the opening of the next.
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
    ///
    /// `output` is `Ok` for normal tool output and `Err` for a failure —
    /// either a dispatch-level error (no sender, channel closed) or a
    /// handler-reported failure. The inner string carries the text in
    /// both cases so UIs can render it; the distinction lets clients
    /// style errors differently and lets agents make retry decisions.
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
    ///
    /// Emitted once per LLM call so clients can track context pressure
    /// in real time and decide when to call `compact_conversation`. The
    /// daemon takes no action on this event — policy is the client's.
    ContextUsage { usage: Usage },
    /// Agent finished with final response.
    Done(AgentResponse),
}

/// Data record of one LLM round (one model call + tool dispatch).
///
/// Carries only what downstream consumers actually read: the assistant
/// message, token usage, the finish reason, and the tool calls / results.
/// No synthesized wire response — the old `AgentStep.response: Response`
/// field was a parallel type that only served to hold `usage` and the
/// final text content. Those two fields are now on the step directly.
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
    /// Error during execution.
    Error(String),
}

impl std::fmt::Display for AgentStopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextResponse => write!(f, "text_response"),
            Self::MaxIterations => write!(f, "max_iterations"),
            Self::NoAction => write!(f, "no_action"),
            Self::Error(msg) => write!(f, "error: {msg}"),
        }
    }
}

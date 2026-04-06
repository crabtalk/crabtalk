//! Agent event types for step-based execution and streaming.
//!
//! Two-level design:
//! - [`AgentStep`]: data record of one LLM round (response + tool dispatch).
//! - [`AgentEvent`]: fine-grained streaming enum for real-time UI updates.
//! - [`AgentResponse`]: final result after a full agent run.
//! - [`AgentStopReason`]: why the agent stopped.

use crate::model::{Message, Response, ToolCall};

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
    ToolResult {
        /// The tool call ID this result belongs to.
        call_id: String,
        /// The output from the tool.
        output: String,
        /// Wall-clock duration of the tool dispatch in milliseconds.
        duration_ms: u64,
    },
    /// All tools completed, continuing to next iteration.
    ToolCallsComplete,
    /// User steering message injected at turn boundary.
    UserSteered { content: String },
    /// Context was compacted — carries the compaction summary.
    Compact { summary: String },
    /// Agent finished with final response.
    Done(AgentResponse),
}

/// Data record of one LLM round (one model call + tool dispatch).
#[derive(Debug, Clone)]
pub struct AgentStep {
    /// The model's response for this step.
    pub response: Response,
    /// Tool calls made in this step (if any).
    pub tool_calls: Vec<ToolCall>,
    /// Results from tool executions as messages.
    pub tool_results: Vec<Message>,
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

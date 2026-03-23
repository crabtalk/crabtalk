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
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Text content delta from the model.
    TextDelta(String),
    /// Thinking/reasoning content delta from the model.
    ThinkingDelta(String),
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
    },
    /// All tools completed, continuing to next iteration.
    ToolCallsComplete,
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

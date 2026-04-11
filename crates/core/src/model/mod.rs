//! Unified LLM interface types and the `Model<P>` wrapper.
//!
//! Thin re-export layer over `crabllm_core` for the core wire types
//! (`Message`, `Tool`, `ToolCall`, `Usage`, …) plus crabtalk's own
//! `HistoryEntry` wrapper and streaming `MessageBuilder`. `Model<P>` is the
//! single seam between crabtalk and any `crabllm_core::Provider`.

pub use builder::MessageBuilder;
pub use client::Model;
pub use crabllm_core::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, CompletionTokensDetails,
    FinishReason, FunctionCall, FunctionDef, Message, Role, Tool, ToolCall, ToolCallDelta,
    ToolChoice, ToolType, Usage,
};
pub use history::{HistoryEntry, estimate_tokens as estimate_history_tokens};
pub use limits::default_context_limit;

pub mod builder;
mod client;
mod history;
mod limits;

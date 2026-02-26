//! Unified LLM interface types and traits.
//!
//! This crate provides the shared types used across all LLM providers:
//! `Message`, `Response`, `StreamChunk`, `Tool`, `Config`, and the `LLM` trait.

pub use config::{Config, General};
pub use message::{Message, Role, estimate_tokens};
pub use noop::NoopProvider;
pub use provider::LLM;
pub use reqwest::{self, Client};
pub use response::{FinishReason, Response, Usage};
pub use stream::StreamChunk;
pub use tool::{FunctionCall, Tool, ToolCall, ToolChoice};

mod config;
mod message;
mod noop;
mod provider;
mod response;
mod stream;
mod tool;

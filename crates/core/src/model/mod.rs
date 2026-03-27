//! Unified LLM interface types and traits.
//!
//! Provides the shared types used across all LLM providers:
//! `Message`, `Response`, `StreamChunk`, `Tool`, `Request`, and the `Model` trait.

use anyhow::Result;
use futures_core::Stream;
pub use limits::default_context_limit;
pub use message::{Message, MessageBuilder, Role, estimate_tokens};
pub use request::Request;
pub use response::{
    Choice, CompletionMeta, CompletionTokensDetails, Delta, FinishReason, Response, Usage,
};
pub use stream::StreamChunk;
pub use tool::{FunctionCall, Tool, ToolCall, ToolChoice};

mod limits;
mod message;
mod request;
mod response;
mod stream;
mod tool;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_model;

/// Unified LLM provider trait.
///
/// Abstracts any LLM provider — single-backend (Claude, GPT) or
/// multi-model registry (ProviderRegistry). All implementations take
/// `&Request` directly; no associated config type.
///
/// Constructors are inherent methods on each provider — never called
/// polymorphically.
pub trait Model: Sized + Clone {
    /// Send a chat completion request.
    fn send(&self, request: &Request) -> impl Future<Output = Result<Response>> + Send;

    /// Stream a chat completion response.
    fn stream(&self, request: Request) -> impl Stream<Item = Result<StreamChunk>> + Send;

    /// Resolve the context limit for a model name.
    ///
    /// Default implementation uses the static prefix-matching map.
    fn context_limit(&self, model: &str) -> usize {
        default_context_limit(model)
    }

    /// Get the active/default model name.
    fn active_model(&self) -> String;
}

/// `()` as a no-op Model for testing (panics on send/stream).
impl Model for () {
    async fn send(&self, _request: &Request) -> Result<Response> {
        panic!("NoopModel::send called — not intended for real LLM calls");
    }

    #[allow(unreachable_code)]
    fn stream(&self, _request: Request) -> impl Stream<Item = Result<StreamChunk>> + Send {
        panic!("NoopModel::stream called — not intended for real LLM calls");
        async_stream::stream! {
            yield Err(anyhow::anyhow!("not implemented"));
        }
    }

    fn context_limit(&self, _model: &str) -> usize {
        0
    }

    fn active_model(&self) -> String {
        String::new()
    }
}

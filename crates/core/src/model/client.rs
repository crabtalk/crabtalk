//! `Model<P>` — newtype wrapper around an `Arc<P>` provider.
//!
//! Exposes `send` / `stream` over `crabllm_core` wire types. The wrapper
//! earns its keep with two things the raw `Provider` trait doesn't do:
//!
//! 1. **Sets `stream: Some(true)` on streaming requests.** Without this flag,
//!    OpenAI-shaped endpoints return a single non-SSE JSON response, the SSE
//!    parser in crabllm-provider sees no `data:` prefixes, the byte stream
//!    completes with zero chunks, and the agent loop yields a Done event
//!    with empty content — "send a message, no reply, no error".
//! 2. **Formats provider errors** with `format_provider_error` so the
//!    upstream failure reason surfaces in the anyhow Display chain instead
//!    of being buried in the error source.
//!
//! Cloning is cheap: `Model<P>` is `Arc<P>` internally, so `clone()` is one
//! refcount bump regardless of `P`. This lets `Runtime` hold a single Model
//! and clone it into every Agent at build time without any `P: Clone` bound.

use anyhow::Result;
use async_stream::try_stream;
use crabllm_core::{
    ApiError, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, Provider,
};
use futures_core::Stream;
use futures_util::StreamExt;
use std::sync::Arc;

/// A wcore-typed view over a `crabllm_core::Provider`.
///
/// Holds an `Arc<P>` so cloning is structural and `P` itself does not need
/// to implement `Clone`. The `'static` bound on `P` flows from the
/// streaming path: `P::chat_completion_stream` returns a `BoxStream<'static>`
/// whose construction requires the implementor to be `'static`. Baking
/// the bound into the struct definition lets every downstream impl
/// (`Agent<P>`, `Runtime<P,H>`) carry the same constraint without
/// repeating it on every method.
pub struct Model<P: Provider + 'static> {
    inner: Arc<P>,
}

impl<P: Provider + 'static> Model<P> {
    /// Wrap a provider in a `Model`. The provider is moved into a new
    /// `Arc`; use [`Model::from_arc`] if you already have one.
    pub fn new(provider: P) -> Self {
        Self {
            inner: Arc::new(provider),
        }
    }

    /// Wrap an existing `Arc<P>` without re-allocating.
    pub fn from_arc(provider: Arc<P>) -> Self {
        Self { inner: provider }
    }

    /// Send a non-streaming chat completion request.
    ///
    /// Sets `stream: Some(false)` on the request and formats provider errors
    /// through `format_provider_error` so the root cause surfaces in the
    /// anyhow Display chain.
    pub async fn send_ct(&self, request: ChatCompletionRequest) -> Result<ChatCompletionResponse> {
        let mut req = request;
        req.stream = Some(false);
        let model_label = req.model.clone();
        self.inner
            .chat_completion(&req)
            .await
            .map_err(|e| format_provider_error(&model_label, "send", e))
    }

    /// Stream a chat completion response.
    ///
    /// The returned stream owns a clone of the provided request and the
    /// inner Arc, so it is `'static` and can be spawned freely. Sets
    /// `stream: Some(true)` — see module docs for why this is load-bearing.
    pub fn stream_ct(
        &self,
        request: ChatCompletionRequest,
    ) -> impl Stream<Item = Result<ChatCompletionChunk>> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        let mut req = request;
        req.stream = Some(true);
        let model_label = req.model.clone();
        try_stream! {
            let mut stream = inner
                .chat_completion_stream(&req)
                .await
                .map_err(|e| format_provider_error(&model_label, "stream open", e))?;
            while let Some(chunk) = stream.next().await {
                yield chunk
                    .map_err(|e| format_provider_error(&model_label, "stream chunk", e))?;
            }
        }
    }
}

impl<P: Provider + 'static> Clone for Model<P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<P: Provider + 'static> std::fmt::Debug for Model<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model").finish()
    }
}

/// Convert a `crabllm_core::Error` into an `anyhow::Error` with a
/// human-readable message that includes the upstream's actual failure
/// reason. For `Error::Provider { status, body }`, attempts to parse the
/// body as an OpenAI-shaped `ApiError` and extract `error.message`; falls
/// back to the truncated raw body. Other variants use the upstream
/// Display impl directly.
///
/// This matters because anyhow's `with_context` only shows the outermost
/// context message on a default `{e}` Display — the root cause lives in
/// the source chain and is invisible to callers that don't explicitly
/// format `{e:#}`. Inlining the root cause into a single message means
/// any surface (TUI, daemon log, etc.) sees the actual failure reason
/// whether or not it walks the error chain.
fn format_provider_error(model: &str, op: &str, e: crabllm_core::Error) -> anyhow::Error {
    match e {
        crabllm_core::Error::Provider { status, body } => {
            let msg = serde_json::from_str::<ApiError>(&body)
                .map(|api_err| api_err.error.message)
                .unwrap_or_else(|_| truncate(&body, 200));
            anyhow::anyhow!("model {op} failed for '{model}' (HTTP {status}): {msg}")
        }
        other => anyhow::anyhow!("model {op} failed for '{model}': {other}"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

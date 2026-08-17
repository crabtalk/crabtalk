//! Unified LLM interface types and the `Model<P>` wrapper.

pub use crabllm_core::{anthropic, codec::MessageBuilder};

use anyhow::Result;
use async_stream::try_stream;
use crabllm_core::Provider;
use futures_core::Stream;
use futures_util::StreamExt;
use std::sync::Arc;

/// A wrapper around a `crabllm_core::Provider` that provides a core-typed view.
pub struct Model<P: Provider + 'static> {
    inner: Arc<P>,
}

impl<P: Provider + 'static> Model<P> {
    /// Wrap a provider in a `Model`.
    pub fn new(provider: P) -> Self {
        Self {
            inner: Arc::new(provider),
        }
    }

    /// Wrap an existing `Arc<P>` without re-allocating.
    pub fn from_arc(provider: Arc<P>) -> Self {
        Self { inner: provider }
    }

    /// Send a non-streaming Anthropic messages request.
    pub async fn send(&self, request: anthropic::Request) -> Result<anthropic::Response> {
        let model = request.model.clone();
        self.inner.anthropic_messages(&request).await.map_err(|e| {
            tracing::warn!(model = %model, op = "send", error = %e, "provider request failed");
            anyhow::anyhow!(e.message())
        })
    }

    /// Stream an Anthropic messages response.
    pub fn stream(
        &self,
        request: anthropic::Request,
    ) -> impl Stream<Item = Result<anthropic::StreamEvent>> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        let mut req = request;
        req.stream = Some(true);
        let model = req.model.clone();
        try_stream! {
            let mut stream = inner
                .anthropic_messages_stream(&req)
                .await
                .map_err(|e| {
                    tracing::warn!(model = %model, op = "stream open", error = %e, "provider request failed");
                    anyhow::anyhow!(e.message())
                })?;
            while let Some(chunk) = stream.next().await {
                yield chunk.map_err(|e| {
                    tracing::warn!(model = %model, op = "stream chunk", error = %e, "provider stream failed");
                    anyhow::anyhow!(e.message())
                })?;
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

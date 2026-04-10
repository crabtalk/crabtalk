//! `Retrying<P>` — a `Provider` wrapper that adds exponential-backoff retry
//! and per-call timeout on top of any inner provider.
//!
//! This restores the retry/timeout semantics that lived in the old
//! `crates/model::Provider` wrapper before the trait migration. It is a
//! deployment-layer concern owned by the daemon — `wcore::Model<P>` does
//! not retry, since not every consumer (e.g. an in-process MLX provider)
//! wants the same retry policy.
//!
//! Per-provider retry config (`max_retries` / `timeout` on individual
//! `ProviderDef` entries) is not threaded through yet; the wrapper applies
//! a single set of defaults to every dispatch. Restoring per-provider
//! config is a follow-up — see TODO below.

use crabllm_core::{
    AudioSpeechRequest, BoxStream, ChatCompletionChunk, ChatCompletionRequest,
    ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse, Error, ImageRequest,
    MultipartField, Provider,
};
use rand::Rng;
use std::time::Duration;

/// Default values matching the old `crates/model::Provider` defaults.
const DEFAULT_MAX_RETRIES: u32 = 2;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);

/// A `Provider` wrapper that retries transient failures with exponential
/// backoff and full jitter, and bounds each attempt with a per-call timeout.
///
/// **Scope:** the retry policy applies to `chat_completion` only. Streaming
/// (`chat_completion_stream`) skips retry — the connection is already
/// established and clients consuming chunk-by-chunk handle their own
/// resumption — but still bounds the connection-establishment phase with
/// the same timeout. The non-chat methods (`embedding`, `image_generation`,
/// `audio_speech`, `audio_transcription`) are bare pass-throughs without
/// retry or timeout, because the daemon's current protocol doesn't expose
/// these endpoints. If a future daemon feature needs them, extend this
/// wrapper's scope at that point.
#[derive(Debug, Clone)]
pub struct Retrying<P: Provider> {
    inner: P,
    max_retries: u32,
    timeout: Duration,
}

impl<P: Provider> Retrying<P> {
    /// Wrap a provider with the default retry policy
    /// (2 retries, 30s timeout, 100ms initial backoff).
    pub fn new(inner: P) -> Self {
        Self {
            inner,
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    // TODO: per-provider retry config. The old crates/model wrapper read
    // `ProviderDef.max_retries` and `ProviderDef.timeout` per provider.
    // Restoring that requires either a config-aware wrapper at the
    // Deployment level (in crabllm) or a per-call config struct.
}

impl<P: Provider> Provider for Retrying<P> {
    async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, Error> {
        let mut backoff = INITIAL_BACKOFF;
        let mut last_err = None;
        for _ in 0..=self.max_retries {
            let result = if self.timeout.is_zero() {
                self.inner.chat_completion(request).await
            } else {
                match tokio::time::timeout(self.timeout, self.inner.chat_completion(request)).await
                {
                    Ok(r) => r,
                    Err(_) => Err(Error::Timeout),
                }
            };
            match result {
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_transient() => {
                    last_err = Some(e);
                    tokio::time::sleep(jittered(backoff)).await;
                    backoff *= 2;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.expect("retry loop exited without producing an error"))
    }

    async fn chat_completion_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<ChatCompletionChunk, Error>>, Error> {
        // Streaming does not retry — connection establishment is the only
        // failure mode we could meaningfully retry, and chunks already
        // streaming would be lost on retry. Apply the timeout to stream
        // open only.
        if self.timeout.is_zero() {
            self.inner.chat_completion_stream(request).await
        } else {
            match tokio::time::timeout(self.timeout, self.inner.chat_completion_stream(request))
                .await
            {
                Ok(r) => r,
                Err(_) => Err(Error::Timeout),
            }
        }
    }

    async fn embedding(&self, request: &EmbeddingRequest) -> Result<EmbeddingResponse, Error> {
        self.inner.embedding(request).await
    }

    async fn image_generation(
        &self,
        request: &ImageRequest,
    ) -> Result<(bytes::Bytes, String), Error> {
        self.inner.image_generation(request).await
    }

    async fn audio_speech(
        &self,
        request: &AudioSpeechRequest,
    ) -> Result<(bytes::Bytes, String), Error> {
        self.inner.audio_speech(request).await
    }

    async fn audio_transcription(
        &self,
        model: &str,
        fields: &[MultipartField],
    ) -> Result<(bytes::Bytes, String), Error> {
        self.inner.audio_transcription(model, fields).await
    }
}

/// Full jitter: random duration in [backoff/2, backoff].
fn jittered(backoff: Duration) -> Duration {
    let lo = backoff.as_millis() as u64 / 2;
    let hi = backoff.as_millis() as u64;
    if lo >= hi {
        return backoff;
    }
    Duration::from_millis(rand::rng().random_range(lo..=hi))
}

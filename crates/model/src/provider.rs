//! Provider implementation backed by crabllm-provider.
//!
//! Wraps `crabllm_provider::Provider` behind wcore's `Model` trait with
//! type conversion and retry logic.

use crate::{config::ProviderDef, convert};
use anyhow::Result;
use async_stream::try_stream;
use crabllm_core::ApiError;
use crabllm_provider::Provider as CtProvider;
use futures_core::Stream;
use futures_util::StreamExt;
use rand::Rng;
use std::time::Duration;
use wcore::model::{Model, Response, StreamChunk};

/// Unified LLM provider wrapping a crabtalk provider instance.
#[derive(Clone)]
pub struct Provider {
    inner: CtProvider,
    client: reqwest::Client,
    model: String,
    max_retries: u32,
    timeout: Duration,
}

/// Strip known endpoint suffixes so both bare origins and full paths work.
fn normalize_base_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    for suffix in ["/chat/completions", "/messages", "/embeddings"] {
        if let Some(stripped) = url.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    url.to_string()
}

/// Construct a `Provider` from a provider definition and model name.
pub fn build_provider(def: &ProviderDef, model: &str, client: reqwest::Client) -> Result<Provider> {
    let mut config = def.clone();
    config.kind = config.effective_kind();
    let mut inner = CtProvider::from(&config);

    // Apply crabtalk-specific base_url normalization (strip endpoint suffixes).
    if let CtProvider::Openai {
        ref mut base_url, ..
    } = inner
    {
        *base_url = normalize_base_url(base_url);
    }

    Ok(Provider {
        inner,
        client,
        model: model.to_owned(),
        max_retries: def.max_retries.unwrap_or(2),
        timeout: Duration::from_secs(def.timeout.unwrap_or(30)),
    })
}

impl Model for Provider {
    async fn send(&self, request: &wcore::model::Request) -> Result<Response> {
        let mut ct_req = convert::to_ct_request(request);
        ct_req.stream = Some(false);
        send_with_retry(
            &self.inner,
            &self.client,
            &ct_req,
            self.max_retries,
            self.timeout,
        )
        .await
    }

    fn stream(
        &self,
        request: wcore::model::Request,
    ) -> impl Stream<Item = Result<StreamChunk>> + Send {
        let inner = self.inner.clone();
        let client = self.client.clone();
        let timeout = self.timeout;
        try_stream! {
            let mut ct_req = convert::to_ct_request(&request);
            ct_req.stream = Some(true);

            let boxed = tokio::time::timeout(timeout, inner.chat_completion_stream(&client, &ct_req))
                .await
                .map_err(|_| anyhow::anyhow!("stream connection timed out"))?
                .map_err(format_provider_error)?;

            let mut stream = std::pin::pin!(boxed);
            while let Some(chunk) = stream.next().await {
                let ct_chunk = chunk.map_err(format_provider_error)?;
                yield convert::from_ct_chunk(ct_chunk);
            }
        }
    }

    fn context_limit(&self, model: &str) -> usize {
        wcore::model::default_context_limit(model)
    }

    fn active_model(&self) -> String {
        self.model.clone()
    }
}

/// Send a non-streaming request with exponential backoff retry on transient errors.
async fn send_with_retry(
    provider: &CtProvider,
    client: &reqwest::Client,
    request: &crabllm_core::ChatCompletionRequest,
    max_retries: u32,
    timeout: Duration,
) -> Result<Response> {
    let mut backoff = Duration::from_millis(100);
    let mut last_err = None;

    for _ in 0..=max_retries {
        let result = if timeout.is_zero() {
            provider.chat_completion(client, request).await
        } else {
            tokio::time::timeout(timeout, provider.chat_completion(client, request))
                .await
                .map_err(|_| crabllm_core::Error::Timeout)?
        };

        match result {
            Ok(resp) => return Ok(convert::from_ct_response(resp)),
            Err(e) if e.is_transient() => {
                last_err = Some(e);
                let jitter = jittered(backoff);
                tokio::time::sleep(jitter).await;
                backoff *= 2;
            }
            Err(e) => return Err(format_provider_error(e)),
        }
    }

    Err(format_provider_error(last_err.unwrap()))
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

/// Convert a crabllm error into an anyhow error with a human-readable message.
///
/// For provider HTTP errors, attempts to parse the response body as an
/// OpenAI-compatible API error and extract the `message` field.
fn format_provider_error(e: crabllm_core::Error) -> anyhow::Error {
    match e {
        crabllm_core::Error::Provider { status, body } => {
            let msg = serde_json::from_str::<ApiError>(&body)
                .map(|api_err| api_err.error.message)
                .unwrap_or_else(|_| truncate(&body, 200));
            anyhow::anyhow!("provider error (HTTP {status}): {msg}")
        }
        other => anyhow::anyhow!("{other}"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}

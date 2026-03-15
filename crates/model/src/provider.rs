//! Provider implementation.
//!
//! Unified `Provider` enum with enum dispatch over concrete backends.
//! `build_provider()` constructs the appropriate variant based on `ApiStandard`.

use crate::{
    config::{ApiStandard, ProviderDef},
    remote::{
        claude::{self, Claude},
        openai::{self, OpenAI},
    },
};
use anyhow::Result;
use async_stream::try_stream;
use compact_str::CompactString;
use futures_core::Stream;
use futures_util::StreamExt;
use wcore::model::{Model, Response, StreamChunk};

/// Unified LLM provider enum.
///
/// The gateway constructs the appropriate variant based on `ApiStandard`
/// from the provider config.
#[derive(Clone)]
pub enum Provider {
    /// OpenAI-compatible API (covers OpenAI, DeepSeek, Grok, Qwen, Kimi, Ollama).
    OpenAI(OpenAI),
    /// Anthropic Messages API.
    Claude(Claude),
}

/// Construct a `Provider` from a provider definition and model name.
///
/// Uses `effective_standard()` to pick the API protocol (OpenAI or Anthropic).
pub async fn build_provider(
    def: &ProviderDef,
    model: &str,
    client: reqwest::Client,
) -> Result<Provider> {
    let api_key = def.api_key.as_deref().unwrap_or("");

    match def.effective_standard() {
        ApiStandard::Anthropic => {
            let url = def.base_url.as_deref().unwrap_or(claude::ENDPOINT);
            Ok(Provider::Claude(Claude::custom(
                client, api_key, url, model,
            )?))
        }
        ApiStandard::OpenAI => {
            let url = def.base_url.as_deref().unwrap_or(openai::endpoint::OPENAI);
            let provider = if api_key.is_empty() {
                OpenAI::no_auth(client, url, model)
            } else {
                OpenAI::custom(client, api_key, url, model)?
            };
            Ok(Provider::OpenAI(provider))
        }
    }
}

impl Model for Provider {
    async fn send(&self, request: &wcore::model::Request) -> Result<Response> {
        match self {
            Self::OpenAI(p) => p.send(request).await,
            Self::Claude(p) => p.send(request).await,
        }
    }

    fn stream(
        &self,
        request: wcore::model::Request,
    ) -> impl Stream<Item = Result<StreamChunk>> + Send {
        let this = self.clone();
        try_stream! {
            match this {
                Provider::OpenAI(p) => {
                    let mut stream = std::pin::pin!(p.stream(request));
                    while let Some(chunk) = stream.next().await {
                        yield chunk?;
                    }
                }
                Provider::Claude(p) => {
                    let mut stream = std::pin::pin!(p.stream(request));
                    while let Some(chunk) = stream.next().await {
                        yield chunk?;
                    }
                }
            }
        }
    }

    fn context_limit(&self, model: &str) -> usize {
        wcore::model::default_context_limit(model)
    }

    fn active_model(&self) -> CompactString {
        match self {
            Self::OpenAI(p) => p.active_model(),
            Self::Claude(p) => p.active_model(),
        }
    }
}

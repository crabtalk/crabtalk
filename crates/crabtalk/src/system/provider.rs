//! The provider crabtalk runs against when the embedder doesn't supply one.

use crabllm_core::{
    AnthropicRequest, AnthropicResponse, AnthropicStreamEvent, BoxStream, ChatCompletionChunk,
    ChatCompletionRequest, ChatCompletionResponse, Error, GeminiRequest, GeminiResponse, ModelList,
    Provider, ProviderConfig,
};
use crabllm_provider::{RemoteProvider, make_client};
use wcore::LlmConfig;

/// Two variants, not one. `Gateway` is a crabllm proxy: it reads the dialect
/// each model reports and routes natively, on top of the SDK's retry and
/// caching path. `Direct` is a single upstream named by `llm.kind`, reached
/// through crabllm's own per-dialect providers. Still one endpoint either way
/// — multiplexing belongs to the embedder, via `CrabTalk::start_with`.
#[derive(Debug, Clone)]
pub enum DefaultProvider {
    Gateway(crabllm_sdk::Client),
    Direct(RemoteProvider),
}

impl From<&LlmConfig> for DefaultProvider {
    fn from(llm: &LlmConfig) -> Self {
        let Some(kind) = llm.kind.clone() else {
            return Self::Gateway(crabllm_sdk::Client::new(
                llm.base_url.clone(),
                llm.api_key.clone(),
            ));
        };

        let config = ProviderConfig {
            kind: Some(kind.clone()),
            api_key: Some(llm.api_key.clone()),
            base_url: (!llm.base_url.is_empty()).then(|| llm.base_url.clone()),
            ..Default::default()
        };
        Self::Direct(RemoteProvider::new(kind.as_str(), &config, make_client()))
    }
}

impl DefaultProvider {
    /// Model ids the endpoint advertises. Empty on failure — logged, never
    /// fatal, because the next reload retries and a daemon that can't list
    /// models can still be pointed at one by name.
    pub async fn model_ids(&self) -> Vec<String> {
        match self.models().await {
            Ok(list) => list.data.into_iter().map(|m| m.id).collect(),
            Err(e) => {
                tracing::warn!("failed to list models from the configured endpoint: {e}");
                Vec::new()
            }
        }
    }
}

impl Provider for DefaultProvider {
    async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, Error> {
        match self {
            Self::Gateway(p) => p.chat_completion(request).await,
            Self::Direct(p) => p.chat_completion(request).await,
        }
    }

    async fn chat_completion_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<ChatCompletionChunk, Error>>, Error> {
        match self {
            Self::Gateway(p) => p.chat_completion_stream(request).await,
            Self::Direct(p) => p.chat_completion_stream(request).await,
        }
    }

    async fn anthropic_messages(
        &self,
        request: &AnthropicRequest,
    ) -> Result<AnthropicResponse, Error> {
        match self {
            Self::Gateway(p) => p.anthropic_messages(request).await,
            Self::Direct(p) => p.anthropic_messages(request).await,
        }
    }

    async fn anthropic_messages_stream(
        &self,
        request: &AnthropicRequest,
    ) -> Result<BoxStream<'static, Result<AnthropicStreamEvent, Error>>, Error> {
        match self {
            Self::Gateway(p) => p.anthropic_messages_stream(request).await,
            Self::Direct(p) => p.anthropic_messages_stream(request).await,
        }
    }

    async fn gemini_generate_content_stream(
        &self,
        model: &str,
        request: &GeminiRequest,
    ) -> Result<BoxStream<'static, Result<GeminiResponse, Error>>, Error> {
        match self {
            Self::Gateway(p) => p.gemini_generate_content_stream(model, request).await,
            Self::Direct(p) => p.gemini_generate_content_stream(model, request).await,
        }
    }

    async fn models(&self) -> Result<ModelList, Error> {
        match self {
            Self::Gateway(p) => p.models().await,
            Self::Direct(p) => p.models().await,
        }
    }
}

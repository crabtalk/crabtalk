//! The provider crabtalk runs against when the embedder doesn't supply one.

use anyhow::Result;
use crabllm_core::{
    BoxStream, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, Error,
    ModelList, Provider, ProviderConfig, Retrying, anthropic, gemini,
};
use crabllm_provider::{ProviderRegistry, RemoteProvider, make_client};
use std::{collections::HashMap, time::Duration};
use store::LlmConfig;

/// What the install talks to. `Gateway` is a crabllm proxy, reached through
/// the SDK's retry and caching path; `Direct` is the single upstream named
/// by `llm.kind`; `Registry` is the several named in `[providers]`, each
/// request routed on the model it asks for.
#[derive(Debug, Clone)]
pub enum DefaultProvider {
    Gateway(crabllm_sdk::Client),
    Direct(Retrying<RemoteProvider>),
    Registry(ProviderRegistry<RemoteProvider>),
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
        // Wrapped only for the stream idle bound — a silent upstream must not
        // hang a turn forever. Retries and the per-attempt timeout stay off:
        // a direct upstream had neither, and a 30s cap would cut off a long
        // non-streaming completion.
        let direct = Retrying::new(RemoteProvider::new(kind.as_str(), &config, make_client()))
            .max_retries(0)
            .timeout(Duration::ZERO);
        Self::Direct(direct)
    }
}

impl DefaultProvider {
    /// Build what `config` names. Pass it a config [`discover`] has already
    /// been over: a registry routes on the model lists it is built with, and
    /// a provider that named none of its own would route nothing.
    pub fn open(config: &store::Config) -> Result<Self> {
        if config.providers.is_empty() {
            return Ok(Self::from(&config.llm));
        }
        let providers: HashMap<String, ProviderConfig> = config
            .providers
            .iter()
            .map(|(name, provider)| (name.clone(), provider.clone()))
            .collect();
        let registry = ProviderRegistry::from_provider_configs(&providers, &HashMap::new(), |p| p)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self::Registry(registry))
    }
}

/// Resolve `config` into the routing table the install runs on, and answer
/// with every model name in it. A provider that named its own models keeps
/// them; one that didn't has them read from its catalogue.
pub async fn discover(config: &mut store::Config) -> Vec<String> {
    if config.providers.is_empty() {
        let single = DefaultProvider::from(&config.llm);
        return catalogue(&single, "the configured endpoint").await;
    }
    // One client, so every provider probes over the same connection pool.
    let client = make_client();
    for (name, provider) in &mut config.providers {
        if provider.models.is_empty() {
            let remote = RemoteProvider::new(name, provider, client.clone());
            provider.models = catalogue(&remote, name).await;
        }
    }
    config
        .providers
        .values()
        .flat_map(|provider| provider.models.iter().cloned())
        .collect()
}

async fn catalogue(provider: &impl Provider, name: &str) -> Vec<String> {
    match provider.models().await {
        Ok(list) => list.data.into_iter().map(|model| model.id).collect(),
        Err(e) => {
            tracing::warn!("no model list from {name}: {e} — name its models in the config");
            Vec::new()
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
            Self::Registry(p) => p.chat_completion(request).await,
        }
    }

    async fn chat_completion_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<ChatCompletionChunk, Error>>, Error> {
        match self {
            Self::Gateway(p) => p.chat_completion_stream(request).await,
            Self::Direct(p) => p.chat_completion_stream(request).await,
            Self::Registry(p) => p.chat_completion_stream(request).await,
        }
    }

    async fn anthropic_messages(
        &self,
        request: &anthropic::Request,
    ) -> Result<anthropic::Response, Error> {
        match self {
            Self::Gateway(p) => p.anthropic_messages(request).await,
            Self::Direct(p) => p.anthropic_messages(request).await,
            Self::Registry(p) => p.anthropic_messages(request).await,
        }
    }

    async fn anthropic_messages_stream(
        &self,
        request: &anthropic::Request,
    ) -> Result<BoxStream<'static, Result<anthropic::StreamEvent, Error>>, Error> {
        match self {
            Self::Gateway(p) => p.anthropic_messages_stream(request).await,
            Self::Direct(p) => p.anthropic_messages_stream(request).await,
            Self::Registry(p) => p.anthropic_messages_stream(request).await,
        }
    }

    async fn gemini_generate_content_stream(
        &self,
        model: &str,
        request: &gemini::Request,
    ) -> Result<BoxStream<'static, Result<gemini::Response, Error>>, Error> {
        match self {
            Self::Gateway(p) => p.gemini_generate_content_stream(model, request).await,
            Self::Direct(p) => p.gemini_generate_content_stream(model, request).await,
            Self::Registry(p) => p.gemini_generate_content_stream(model, request).await,
        }
    }

    async fn models(&self) -> Result<ModelList, Error> {
        match self {
            Self::Gateway(p) => p.models().await,
            Self::Direct(p) => p.models().await,
            Self::Registry(p) => p.models().await,
        }
    }
}

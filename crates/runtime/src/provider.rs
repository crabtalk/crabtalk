//! Provider enum for static dispatch over LLM implementations.

use anyhow::Result;
use deepseek::DeepSeek;
use futures_core::Stream;
use llm::{Client, General, LLM, Message, Response, StreamChunk};

/// Unified LLM provider (static dispatch, no dyn).
#[derive(Clone)]
pub enum Provider {
    /// DeepSeek provider.
    DeepSeek(DeepSeek),
}

impl Provider {
    /// Create a provider from a model name.
    pub fn new(model: &str, client: Client, key: &str) -> Result<Self> {
        match model {
            s if s.starts_with("deepseek") => Ok(Self::DeepSeek(DeepSeek::new(client, key)?)),
            _ => anyhow::bail!("unknown provider for model: {model}"),
        }
    }

    /// Create a DeepSeek provider with a default HTTP client.
    pub fn deepseek(key: &str) -> Result<Self> {
        Ok(Self::DeepSeek(DeepSeek::new(Client::new(), key)?))
    }

    /// Context window limit for the current provider/model.
    ///
    /// If `config.context_limit` is set, that takes precedence.
    /// Otherwise, model-based defaults are used.
    pub fn context_limit(&self, config: &General) -> usize {
        config.context_limit.unwrap_or(match self {
            Self::DeepSeek(_) => 64_000,
        })
    }
}

impl LLM for Provider {
    type ChatConfig = General;

    fn new(client: Client, key: &str) -> Result<Self> {
        Ok(Self::DeepSeek(DeepSeek::new(client, key)?))
    }

    async fn send(&self, config: &General, messages: &[Message]) -> Result<Response> {
        match self {
            Self::DeepSeek(p) => {
                let cfg = deepseek::Request::from(config.clone());
                p.send(&cfg, messages).await
            }
        }
    }

    fn stream(
        &self,
        config: General,
        messages: &[Message],
        usage: bool,
    ) -> impl Stream<Item = Result<StreamChunk>> {
        match self {
            Self::DeepSeek(p) => {
                let cfg = deepseek::Request::from(config);
                p.stream(cfg, messages, usage)
            }
        }
    }
}

//! `ProviderManager` — concurrent-safe named provider registry with model
//! routing and active-provider swapping.

use crate::{Provider, ProviderConfig, build_provider};
use anyhow::{Result, anyhow, bail};
use async_stream::try_stream;
use compact_str::CompactString;
use futures_core::Stream;
use futures_util::StreamExt;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use wcore::model::{Model, Response, StreamChunk, default_context_limit};

/// Manages a set of named providers with an active selection.
///
/// All methods that read or mutate the inner state acquire the `RwLock`.
/// `active()` returns a clone of the current `Provider` — callers do not
/// hold the lock while performing LLM calls.
pub struct ProviderManager {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    /// Provider instances keyed by model name.
    providers: BTreeMap<CompactString, Provider>,
    /// Model name of the currently active provider.
    active: CompactString,
    /// Shared HTTP client for constructing new providers.
    client: reqwest::Client,
}

/// Info about a single provider entry returned by `list()`.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    /// Provider model name (key).
    pub name: CompactString,
    /// Whether this is the active provider.
    pub active: bool,
}

impl ProviderManager {
    /// Create an empty manager with the given active model name.
    ///
    /// Use `add_provider()` or `add_config()` to populate.
    pub fn new(active: impl Into<CompactString>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                providers: BTreeMap::new(),
                active: active.into(),
                client: reqwest::Client::new(),
            })),
        }
    }

    /// Create a manager from a list of remote provider configs.
    ///
    /// The first element becomes the active provider.
    /// Returns an error if the slice is empty, any config fails validation, or
    /// any provider fails to build.
    pub async fn from_configs(configs: &[ProviderConfig]) -> Result<Self> {
        if configs.is_empty() {
            bail!("at least one provider config is required");
        }
        let manager = Self::new(configs[0].model.clone());
        for config in configs {
            manager.add_config(config).await?;
        }
        Ok(manager)
    }

    /// Add a pre-built provider directly (e.g. local models from registry).
    pub fn add_provider(&self, name: impl Into<CompactString>, provider: Provider) -> Result<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        inner.providers.insert(name.into(), provider);
        Ok(())
    }

    /// Add a remote provider from config. Validates and builds it.
    pub async fn add_config(&self, config: &ProviderConfig) -> Result<()> {
        config.validate()?;
        let client = {
            let inner = self
                .inner
                .read()
                .map_err(|_| anyhow!("provider lock poisoned"))?;
            inner.client.clone()
        };
        let provider = build_provider(config, client).await?;
        let mut inner = self
            .inner
            .write()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        inner.providers.insert(config.model.clone(), provider);
        Ok(())
    }

    /// Get a clone of the active provider.
    pub fn active(&self) -> Result<Provider> {
        let inner = self
            .inner
            .read()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        Ok(inner.providers[&inner.active].clone())
    }

    /// Get the model name of the active provider (also its key).
    pub fn active_model_name(&self) -> Result<CompactString> {
        let inner = self
            .inner
            .read()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        Ok(inner.active.clone())
    }

    /// Switch to a different provider by model name. Returns an error if the
    /// name is not found.
    pub fn switch(&self, model: &str) -> Result<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        if !inner.providers.contains_key(model) {
            bail!("provider '{}' not found", model);
        }
        inner.active = CompactString::from(model);
        Ok(())
    }

    /// Remove a provider by model name. Fails if the provider is currently
    /// active.
    pub fn remove(&self, model: &str) -> Result<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        if inner.active == model {
            bail!("cannot remove the active provider '{}'", model);
        }
        if inner.providers.remove(model).is_none() {
            bail!("provider '{}' not found", model);
        }
        Ok(())
    }

    /// List all providers with their active status.
    pub fn list(&self) -> Result<Vec<ProviderEntry>> {
        let inner = self
            .inner
            .read()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        Ok(inner
            .providers
            .keys()
            .map(|name| ProviderEntry {
                name: name.clone(),
                active: *name == inner.active,
            })
            .collect())
    }

    /// Look up a provider by model name. Returns a clone so callers don't
    /// hold the lock during LLM calls.
    fn provider_for(&self, model: &str) -> Result<Provider> {
        let inner = self
            .inner
            .read()
            .map_err(|_| anyhow!("provider lock poisoned"))?;
        inner
            .providers
            .get(model)
            .cloned()
            .ok_or_else(|| anyhow!("model '{}' not found in registry", model))
    }

    /// Wait until the active provider is ready.
    ///
    /// No-op for remote providers. For local providers, blocks until the
    /// model finishes loading.
    pub async fn wait_until_ready(&self) -> Result<()> {
        let mut provider = self.active()?;
        provider.wait_until_ready().await
    }

    /// Resolve the context limit for a model.
    ///
    /// Resolution chain: provider reports limit → static map → 8192 default.
    /// Falls back to the static default if the lock is poisoned.
    pub fn context_limit(&self, model: &str) -> usize {
        let Ok(inner) = self.inner.read() else {
            return default_context_limit(model);
        };
        if let Some(provider) = inner.providers.get(model)
            && let Some(limit) = provider.context_length(model)
        {
            return limit;
        }
        default_context_limit(model)
    }
}

impl Model for ProviderManager {
    async fn send(&self, request: &wcore::model::Request) -> Result<Response> {
        let provider = self.provider_for(&request.model)?;
        provider.send(request).await
    }

    fn stream(
        &self,
        request: wcore::model::Request,
    ) -> impl Stream<Item = Result<StreamChunk>> + Send {
        let result = self.provider_for(&request.model);
        try_stream! {
            let provider = result?;
            let mut stream = std::pin::pin!(provider.stream(request));
            while let Some(chunk) = stream.next().await {
                yield chunk?;
            }
        }
    }

    fn context_limit(&self, model: &str) -> usize {
        ProviderManager::context_limit(self, model)
    }

    fn active_model(&self) -> CompactString {
        self.active_model_name()
            .unwrap_or_else(|_| CompactString::const_new("unknown"))
    }
}

impl std::fmt::Debug for ProviderManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.read() {
            Ok(inner) => f
                .debug_struct("ProviderManager")
                .field("active", &inner.active)
                .field("count", &inner.providers.len())
                .finish(),
            Err(_) => f
                .debug_struct("ProviderManager")
                .field("error", &"lock poisoned")
                .finish(),
        }
    }
}

impl Clone for ProviderManager {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

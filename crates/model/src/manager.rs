//! `ProviderManager` — concurrent-safe named provider registry with model
//! routing and active-provider swapping.

use crate::{Provider, ProviderDef, build_provider};
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
    /// Use `add_provider()` or `add_model()` to populate.
    pub fn new(active: impl Into<CompactString>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                providers: BTreeMap::new(),
                active: active.into(),
                client: reqwest::Client::new(),
            })),
        }
    }

    /// Build a manager from a map of provider definitions and an active model.
    ///
    /// Iterates each provider def, building a `Provider` instance per model
    /// in its `models` list.
    pub async fn from_providers(
        active: CompactString,
        providers: &BTreeMap<CompactString, ProviderDef>,
    ) -> Result<Self> {
        let manager = Self::new(active);
        for def in providers.values() {
            manager.add_def(def).await?;
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

    /// Add all models from a provider definition. Builds a `Provider` per model.
    pub async fn add_def(&self, def: &ProviderDef) -> Result<()> {
        let client = {
            let inner = self
                .inner
                .read()
                .map_err(|_| anyhow!("provider lock poisoned"))?;
            inner.client.clone()
        };
        for model_name in &def.models {
            let provider = build_provider(def, model_name, client.clone()).await?;
            let mut inner = self
                .inner
                .write()
                .map_err(|_| anyhow!("provider lock poisoned"))?;
            inner.providers.insert(model_name.clone(), provider);
        }
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

    /// Resolve the context limit for a model.
    ///
    /// Uses the static map in `wcore::model::default_context_limit`.
    pub fn context_limit(&self, model: &str) -> usize {
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

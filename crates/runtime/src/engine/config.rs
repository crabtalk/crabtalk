//! Storage-backed configuration queries — providers, models, active agent.

use super::Runtime;
use crate::Config;
use anyhow::Result;
use wcore::{
    paths,
    protocol::message::{ModelInfo, ProviderInfo, ProviderKind},
    storage::Storage,
};

impl<C: Config> Runtime<C> {
    /// The active model — defined as the default agent's `model` field.
    /// Empty string if the default agent is missing (pre-scaffold).
    pub fn active_model(&self) -> String {
        self.storage()
            .load_agent_by_name(paths::DEFAULT_AGENT)
            .ok()
            .flatten()
            .map(|c| c.model)
            .unwrap_or_default()
    }

    /// Return the provider name that owns the given model, or empty string
    /// if no provider declares it.
    pub fn provider_name_for_model(&self, model: &str) -> String {
        self.storage()
            .load_config()
            .ok()
            .and_then(|c| {
                c.provider
                    .iter()
                    .find(|(_, def)| def.models.iter().any(|m| m == model))
                    .map(|(name, _)| name.clone())
            })
            .unwrap_or_default()
    }

    /// List configured providers with an `active` flag per provider and the
    /// provider's `ProviderDef` serialized as JSON in `ProviderInfo.config`
    /// (the wire protocol carries the def as a JSON blob).
    pub fn list_providers(&self) -> Result<Vec<ProviderInfo>> {
        let config = self.storage().load_config()?;
        let active_model = self.active_model();
        Ok(config
            .provider
            .iter()
            .map(|(name, def)| ProviderInfo {
                name: name.clone(),
                active: !active_model.is_empty() && def.models.contains(&active_model),
                config: serde_json::to_string(def).unwrap_or_default(),
            })
            .collect())
    }

    /// List every model across every provider, with an `active` flag for
    /// the one the default agent uses.
    pub fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let config = self.storage().load_config()?;
        let active_model = self.active_model();
        let mut models = Vec::new();
        for (provider_name, def) in &config.provider {
            let kind: i32 = ProviderKind::from(&def.kind).into();
            for model_name in &def.models {
                models.push(ModelInfo {
                    name: model_name.clone(),
                    provider: provider_name.clone(),
                    active: *model_name == active_model,
                    kind,
                });
            }
        }
        Ok(models)
    }
}

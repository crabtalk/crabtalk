//! Storage-backed configuration queries — active model, model listing.

use super::Runtime;
use crate::Config;
use wcore::{paths, protocol::message::ModelInfo, storage::Storage};

impl<C: Config> Runtime<C> {
    /// The active model — defined as the default agent's `model` field.
    /// Empty string if the default agent is missing (pre-scaffold).
    pub async fn active_model(&self) -> String {
        self.storage()
            .load_agent_by_name(paths::DEFAULT_AGENT)
            .await
            .ok()
            .flatten()
            .map(|c| c.model)
            .unwrap_or_default()
    }

    /// Set the cached model list — called by the daemon builder after
    /// fetching `/v1/models` from the LLM endpoint at startup / reload.
    pub fn set_models(&self, names: Vec<String>) {
        *self.models.write() = names;
    }

    /// List models advertised by the configured LLM endpoint at startup
    /// (or last reload). Flags the currently active model.
    pub async fn list_models(&self) -> Vec<ModelInfo> {
        let active_model = self.active_model().await;
        self.models
            .read()
            .iter()
            .map(|name| ModelInfo {
                name: name.clone(),
                active: *name == active_model,
            })
            .collect()
    }
}

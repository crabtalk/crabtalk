//! Model configuration.
//!
//! `ProviderConfig` and `ApiStandard` are defined in wcore and re-exported here.

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use wcore::config::provider::{ApiStandard, ProviderConfig};

/// Model configuration for the daemon.
///
/// Remote models are configured as flat keys under `[model]` (e.g.
/// `[model.deepseek-chat]`). The active model name lives in
/// `[walrus].model`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    /// Optional embedding model
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<CompactString>,
    /// Remote model configurations, keyed by model name.
    #[serde(flatten)]
    pub remotes: BTreeMap<CompactString, ProviderConfig>,
}

//! Model configuration.
//!
//! `ProviderDef` and `ApiStandard` are defined in wcore and re-exported here.

use anyhow::{Result, bail};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub use wcore::config::provider::{ApiStandard, ProviderDef};

/// Model configuration for the daemon.
///
/// Providers are configured as `[provider.<name>]` sections, each owning
/// a list of model names. The active model name lives in `[system.walrus].model`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    /// Optional embedding model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<CompactString>,
}

impl ModelConfig {
    /// Validate provider definitions and reject duplicate model names.
    pub fn validate(providers: &BTreeMap<CompactString, ProviderDef>) -> Result<()> {
        let mut seen = HashSet::new();
        for (name, def) in providers {
            def.validate(name).map_err(|e| anyhow::anyhow!(e))?;
            for model in &def.models {
                if !seen.insert(model.clone()) {
                    bail!("duplicate model name '{model}' across providers");
                }
            }
        }
        Ok(())
    }
}

use crate::engine::EngineId;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configuration for the meta search engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Which engines to use.
    #[serde(default = "default_engines")]
    pub engines: Vec<EngineId>,

    /// Request timeout per engine in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Maximum results to return.
    #[serde(default = "default_max_results")]
    pub max_results: usize,

    /// Cache TTL in seconds (0 to disable).
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,

    /// Maximum number of entries in the LRU cache.
    #[serde(default = "default_cache_capacity")]
    pub cache_capacity: usize,

    /// Output format.
    #[serde(default)]
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Json,
    Text,
    Compact,
}

fn default_engines() -> Vec<EngineId> {
    EngineId::ALL.to_vec()
}

fn default_timeout() -> u64 {
    10
}

fn default_max_results() -> usize {
    20
}

fn default_cache_ttl() -> u64 {
    300
}

fn default_cache_capacity() -> usize {
    256
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engines: default_engines(),
            timeout_secs: default_timeout(),
            max_results: default_max_results(),
            cache_ttl_secs: default_cache_ttl(),
            cache_capacity: default_cache_capacity(),
            output_format: OutputFormat::default(),
        }
    }
}

impl Config {
    /// Load config from a file path, falling back to defaults for missing fields.
    pub fn load(path: &Path) -> Result<Self, crate::error::Error> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Generate default config as TOML string.
    pub fn default_toml() -> String {
        toml::to_string_pretty(&Config::default()).unwrap_or_default()
    }
}

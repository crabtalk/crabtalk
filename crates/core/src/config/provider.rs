//! Remote provider configuration.

use anyhow::{Result, bail};
use compact_str::CompactString;
use serde::{Deserialize, Serialize};

/// API protocol standard for remote providers.
///
/// Only two wire formats exist: OpenAI-compatible and Anthropic.
/// Defaults to `OpenAI` when omitted in config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiStandard {
    /// OpenAI-compatible chat completions API (covers DeepSeek, Grok, Qwen, Kimi, Ollama, etc.).
    #[default]
    OpenAI,
    /// Anthropic Messages API.
    Anthropic,
}

/// Provider definition — credentials and a list of models served.
///
/// Each `[provider.<name>]` in TOML becomes one `ProviderDef`. The TOML key
/// is the provider name (not stored in the struct). Multiple models share
/// the same credentials and endpoint.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderDef {
    /// API key. Supports `${ENV_VAR}` expansion at the daemon layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL for the provider endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// API protocol standard. Defaults to OpenAI if omitted.
    #[serde(default)]
    pub standard: ApiStandard,
    /// Model names served by this provider.
    #[serde(default)]
    pub models: Vec<CompactString>,
}

impl ProviderDef {
    /// Resolve the effective API standard.
    ///
    /// Returns `Anthropic` if the field is explicitly set to `Anthropic`,
    /// or if `base_url` contains "anthropic". Otherwise `OpenAI`.
    pub fn effective_standard(&self) -> ApiStandard {
        if self.standard == ApiStandard::Anthropic {
            return ApiStandard::Anthropic;
        }
        if let Some(url) = &self.base_url
            && url.contains("anthropic")
        {
            return ApiStandard::Anthropic;
        }
        ApiStandard::OpenAI
    }

    /// Validate field combinations.
    pub fn validate(&self, provider_name: &str) -> Result<()> {
        if self.models.is_empty() {
            bail!("provider '{provider_name}' has no models");
        }
        // api_key is required unless base_url is set (e.g. Ollama).
        if self.api_key.is_none() && self.base_url.is_none() {
            bail!("provider '{provider_name}' requires api_key or base_url");
        }
        Ok(())
    }
}

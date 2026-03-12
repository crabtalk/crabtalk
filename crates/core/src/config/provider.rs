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

/// Remote provider configuration.
///
/// Any model name is valid — the `standard` field (or auto-detection from
/// `base_url`) determines which API protocol to use. Local models are handled
/// by the built-in registry, not by this config.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderConfig {
    /// Model identifier sent to the remote API.
    pub model: CompactString,
    /// API key for remote providers. Supports `${ENV_VAR}` expansion at the
    /// daemon layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL for the remote provider endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// API protocol standard. Defaults to OpenAI if omitted.
    #[serde(default)]
    pub standard: ApiStandard,
}

impl ProviderConfig {
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
    ///
    /// Called on startup and on provider add/reload.
    pub fn validate(&self) -> Result<()> {
        if self.model.is_empty() {
            bail!("model is required");
        }
        // Remote providers: api_key is required unless base_url is set
        // (e.g. Ollama which is keyless with a local base_url).
        if self.api_key.is_none() && self.base_url.is_none() {
            bail!(
                "remote provider '{}' requires api_key or base_url",
                self.model
            );
        }
        Ok(())
    }
}

//! Single-endpoint LLM configuration.
//!
//! One endpoint: a crabllm gateway by default, or the provider named by
//! `kind`. Several at once is [`providers`](crate::Config::providers), and
//! a file that sets both is rejected when it is read.

use crabllm_core::ProviderKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Endpoint origin, e.g. `http://localhost:5632`. Route paths are
    /// appended to it as written, so a direct `anthropic` endpoint needs
    /// the `/v1` — `https://api.anthropic.com/v1`.
    #[serde(default)]
    pub base_url: String,
    /// Bearer token for the endpoint. `${VAR}` is resolved from the
    /// environment when the config is read.
    #[serde(default)]
    pub api_key: String,
    /// Talk to a provider directly instead of through a gateway —
    /// `"anthropic"`, `"deepseek"`, `"ollama"`, or any other kind crabllm
    /// knows. Omitted means the gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ProviderKind>,
}

impl LlmConfig {
    /// Whether the file names an endpoint here at all.
    pub fn is_set(&self) -> bool {
        !self.base_url.is_empty() || !self.api_key.is_empty() || self.kind.is_some()
    }
}

//! Single-endpoint LLM configuration.
//!
//! crabtalk talks to exactly one endpoint — a crabllm gateway by default, or
//! any provider named by `kind`. Model routing is the endpoint's concern; we
//! query it for what's available at startup, and we don't multiplex providers
//! here. An embedder that needs several at once supplies its own provider
//! through `CrabTalk::start_with`.

use crabllm_core::ProviderKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Gateway origin, e.g. `http://localhost:5632` or
    /// `https://api.anthropic.com`. The SDK appends route paths
    /// (`/v1/messages`); a trailing `/v1` is tolerated and stripped.
    #[serde(default)]
    pub base_url: String,
    /// Bearer token for the endpoint. Supports `${ENV_VAR}` interpolation
    /// at load time (resolved by the daemon builder).
    #[serde(default)]
    pub api_key: String,
    /// Talk to a provider directly instead of through a gateway —
    /// `"anthropic"`, `"deepseek"`, `"ollama"`, or any other kind crabllm
    /// knows. Omitted means the gateway, which is the only path that routes
    /// by the dialect each model reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ProviderKind>,
}

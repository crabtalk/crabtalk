//! Single-endpoint LLM configuration.
//!
//! crabtalk talks to exactly one OpenAI-compatible endpoint (typically
//! crabllm, but any compatible gateway works). Model routing is the
//! endpoint's concern — we query `/v1/models` at startup to discover
//! what's available; we don't try to multiplex providers here.

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
}

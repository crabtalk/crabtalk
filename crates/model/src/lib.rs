//! Model crate — LLM provider implementations, enum dispatch, configuration,
//! construction, and runtime management.
//!
//! Merges all provider backends (OpenAI, Claude, Local) with the `Provider`
//! enum, `ProviderManager`, and `ProviderDef` into a single crate.
//! `ProviderDef` describes a provider (api_key, base_url, standard, models).
//! Each `[provider.<name>]` in TOML becomes one `ProviderDef`.

pub mod config;
pub mod manager;
mod provider;

#[path = "../remote/mod.rs"]
pub mod remote;

/// Default model name when none is configured.
pub fn default_model() -> &'static str {
    "deepseek-chat"
}

pub use config::{ApiStandard, ModelConfig, ProviderDef};
pub use manager::ProviderManager;
pub use provider::{Provider, build_provider};
pub use reqwest::Client;

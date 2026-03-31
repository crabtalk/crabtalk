//! Remote provider configuration — re-exported from crabllm-core.

pub use crabllm_core::{ProviderConfig as ProviderDef, ProviderKind as ApiStandard};

/// Provider preset — template for setting up a new provider.
pub struct ProviderPreset {
    /// Preset name (e.g. "anthropic", "openai", "custom").
    pub name: &'static str,
    /// API standard.
    pub kind: ApiStandard,
    /// Default/suggested base URL (editable by user).
    pub base_url: &'static str,
    /// Hardcoded base URL (shown read-only, not saved to config). Empty if editable.
    pub fixed_base_url: &'static str,
    /// Default model name for this provider.
    pub default_model: &'static str,
}

impl ProviderPreset {
    /// Whether the base_url field is editable for this preset.
    pub fn base_url_editable(&self) -> bool {
        self.fixed_base_url.is_empty()
    }
}

pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "anthropic",
        kind: ApiStandard::Anthropic,
        base_url: "",
        fixed_base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-4-5-20250514",
    },
    ProviderPreset {
        name: "openai",
        kind: ApiStandard::Openai,
        base_url: "https://api.openai.com/v1",
        fixed_base_url: "",
        default_model: "gpt-4o",
    },
    ProviderPreset {
        name: "google",
        kind: ApiStandard::Google,
        base_url: "",
        fixed_base_url: "https://generativelanguage.googleapis.com/v1beta",
        default_model: "gemini-2.5-pro",
    },
    ProviderPreset {
        name: "ollama",
        kind: ApiStandard::Ollama,
        base_url: "http://localhost:11434/v1",
        fixed_base_url: "",
        default_model: "llama3",
    },
    ProviderPreset {
        name: "azure",
        kind: ApiStandard::Azure,
        base_url: "",
        fixed_base_url: "",
        default_model: "gpt-4o",
    },
];

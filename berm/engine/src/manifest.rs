//! What a harness says it is.
//!
//! Carried as an ELF section rather than an export, so reading it never means
//! running the guest (RFC 0205). It is parsed once at load: the tool schemas
//! the model sees come from here, and so does the list of capabilities the
//! author wants — which is documentation, not a grant.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The ABI this host speaks. A harness built against a different one is
/// refused rather than dispatched into a capability its author did not mean.
pub const ABI_VERSION: u32 = 0;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub abi_version: u32,
    /// What the harness *wants*. The declaration decides what it gets.
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments, as the model receives it.
    pub parameters: serde_json::Value,
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Self> {
        let manifest: Manifest =
            serde_json::from_str(json).context("harness manifest is not valid JSON")?;
        if manifest.abi_version != ABI_VERSION {
            bail!(
                "harness was built against ABI version {}, this host speaks {ABI_VERSION}",
                manifest.abi_version
            );
        }
        Ok(manifest)
    }
}

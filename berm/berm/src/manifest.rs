//! What a harness says it is.
//!
//! Carried as an ELF section rather than an export, so reading it never means
//! running the guest (RFC 0205). It is parsed once at load: the tool schemas
//! the model sees come from here, and so does the usage an embedder puts in
//! front of a model before it chooses between them.

use anyhow::{Context, Result, bail};
use object::{Object, ObjectSection};
use serde::Deserialize;

use crate::abi;

/// The ABI this host speaks. A harness built against a different one is
/// refused rather than dispatched into a system harness its author did not
/// mean.
pub const ABI_VERSION: u32 = 0;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub abi_version: u32,
    pub tools: Vec<ToolSpec>,
    /// When to reach for these tools, and how they go together — the
    /// question no single tool's `description` answers, because it is about
    /// choosing between them. An embedder puts this in front of a model
    /// before it decides, so it is paid on every turn: a few lines, not a
    /// manual.
    #[serde(default)]
    pub usage: String,
}

impl Manifest {
    /// Pull the manifest out of the ELF. This runs before anything is compiled,
    /// let alone executed — a harness gets to describe itself without being given
    /// a turn.
    /// Read what an ELF claims to be, without compiling or running it.
    ///
    /// This is what the section is *for* (RFC 0205): learning a harness's tools,
    /// wants, and usage must not mean instantiating it. An embedder assembling a
    /// prompt or listing a registry needs exactly this and nothing else.
    pub fn from_elf(elf: &[u8]) -> Result<Self> {
        let file = object::File::parse(elf).context("harness is not a readable ELF")?;
        let section = file
            .section_by_name(abi::ABI_SECTION)
            .with_context(|| format!("harness has no {} section", abi::ABI_SECTION))?;
        let bytes = section.data().context("harness manifest is unreadable")?;
        let json = String::from_utf8(bytes.to_vec()).context("harness manifest is not UTF-8")?;
        Self::parse(&json)
    }

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

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments, as the model receives it.
    pub parameters: serde_json::Value,
}

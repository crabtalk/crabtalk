//! Per-agent harness declaration.
//!
//! An agent owns its harnesses by value, the way it owns its MCPs — a
//! hash-pinned ELF travels better than a `command` + `args` + `env` triple
//! that assumes the destination machine already has the binary (RFC 0205).
//!
//! The declaration is the grant. What the manifest asks for is documentation;
//! what is written here is what the harness gets, and the daemon never infers
//! one from the other.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessConfig {
    /// Harness name. Its image is `{name}.elf` under the harnesses directory.
    pub name: String,

    /// System harnesses this one may reach. A name absent here is absent from
    /// the linker it is instantiated with, and that absence is the enforcement
    /// — there is no check to write and none to forget.
    pub system: Vec<String>,

    /// The subtree `fs` and `exec` are bounded by, and the default working
    /// directory for a call that names none.
    ///
    /// This is the argument to those two, and the grant *is* the argument:
    /// without a root neither is registered, so an under-specified declaration
    /// reaches nothing rather than everything.
    pub root: Option<Root>,

    /// Hosts `http` may reach, matched exactly and case-insensitively.
    ///
    /// What `root` is to `fs`, this is to `http`: the argument the grant
    /// consists of. An empty list leaves it unregistered, so `http` without
    /// hosts reaches nothing.
    ///
    /// It bounds `http`, not the harness. `exec` is a shell and a shell has
    /// curl, so a declaration granting both has egress this list says nothing
    /// about — the two are not additive, `exec` is simply the wider one.
    pub hosts: Vec<String>,
}

/// Where the bound on `fs` and `exec` comes from.
///
/// Both variants carry the bound itself, so a session narrowing within one can
/// never widen it: the clamp is the type rather than a check somewhere that can
/// be forgotten. Absent this whole value, neither harness is constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Root {
    /// Bound to whatever the session named, resolved inside this path. A
    /// session that named none is bound to this path itself.
    Session(PathBuf),
    /// Bound to this path, whatever the session named.
    // Untagged so a declaration written before sessions could narrow — a bare
    // `root = "/path"` — still reads as the fixed grant it was.
    #[serde(untagged)]
    Fixed(PathBuf),
}

impl Root {
    /// The outer bound, which nothing resolved against this may escape.
    pub fn bound(&self) -> &Path {
        match self {
            Self::Session(path) | Self::Fixed(path) => path,
        }
    }
}

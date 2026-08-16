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
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessConfig {
    /// Harness name. Its image is `{name}.elf` under the harnesses directory.
    pub name: String,

    /// Capabilities granted to it. A name absent here is absent from the
    /// linker the harness is instantiated with, and that absence is the
    /// enforcement — there is no check to write and none to forget.
    pub capabilities: Vec<String>,

    /// The subtree `fs` and `exec` are bounded by, and the default working
    /// directory for a call that names none.
    ///
    /// This is the argument to those capabilities, and the grant *is* the
    /// argument: without a root neither is registered, so an under-specified
    /// declaration reaches nothing rather than everything.
    pub root: Option<PathBuf>,

    /// Hosts `http` may reach, matched exactly and case-insensitively.
    ///
    /// What `root` is to `fs`, this is to `http`: the argument the grant
    /// consists of. An empty list leaves the capability unregistered, so
    /// `http` without hosts reaches nothing — and since a name is only
    /// reachable by being written here, the daemon's own port is out of reach
    /// unless someone names it.
    pub hosts: Vec<String>,
}

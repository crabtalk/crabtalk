//! Per-agent harness declaration.
//!
//! An agent owns its harnesses by value, the way it owns its MCPs — a
//! hash-pinned ELF travels better than a `command` + `args` + `env` triple
//! that assumes the destination machine already has the binary (RFC 0205).
//!
//! The image is the grant. A harness that does not call a system harness does
//! not need to be stopped from calling it, and one that does is a harness for
//! exactly that — so what bounds a declaration is its arguments, not a list.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessConfig {
    /// Harness name. Its image is `{name}.elf` under the harnesses directory.
    pub name: String,

    /// The subtree `fs` and `exec` are bounded by, and the default working
    /// directory for a call that names none.
    ///
    /// The grant *is* the argument: without a root neither is registered, so an
    /// under-specified declaration reaches nothing rather than everything.
    pub root: Option<Root>,

    /// Hosts `http` may reach, matched exactly and case-insensitively.
    ///
    /// What `root` is to `fs`, this is to `http`. An empty list leaves it
    /// unregistered, so `http` without hosts reaches nothing.
    ///
    /// It bounds `http`, not the harness. `exec` is a shell and a shell has
    /// curl, so a declaration with a root has egress this list says nothing
    /// about — the two are not additive, `exec` is simply the wider one.
    pub hosts: Vec<String>,
}

/// Where the bound on `fs` and `exec` comes from.
///
/// A session narrowing within a bound can never widen it: the clamp is the type
/// rather than a check somewhere that can be forgotten. Absent this whole value,
/// neither harness is constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Root {
    /// Bound to whatever the session named, resolved inside this path. A
    /// session that named none is bound to this path itself.
    Session(PathBuf),
    /// [`Root::Session`] under the home directory of whoever is running, read
    /// where the harness binds. The path is absent here because a stored one is
    /// the home of the machine that first scaffolded the agent.
    Home,
    /// Bound to this path, whatever the session named.
    // Untagged so a declaration written before sessions could narrow — a bare
    // `root = "/path"` — still reads as the fixed grant it was.
    #[serde(untagged)]
    Fixed(PathBuf),
}

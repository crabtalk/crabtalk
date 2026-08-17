//! Short-name → crate/binary resolution.

/// What a binary is to the user, and therefore which verb installs it.
/// Mirrors the workspace split: `apps/` are the things you run, `harness/`
/// are the services you attach to a running system.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    App,
    Harness,
}

impl Kind {
    /// What this kind is called in prose.
    pub fn noun(self) -> &'static str {
        match self {
            Self::App => "crabtalk app",
            Self::Harness => "harness service",
        }
    }

    /// The verb that installs this kind.
    pub fn install_verb(self) -> &'static str {
        match self {
            Self::App => "install",
            Self::Harness => "add",
        }
    }

    /// The verb that removes it again.
    pub fn remove_verb(self) -> &'static str {
        match self {
            Self::App => "uninstall",
            Self::Harness => "remove",
        }
    }
}

/// A first-party crabtalk binary that crabup knows about.
pub struct Entry {
    /// Short name used on the crabup CLI (`daemon`, `cli`, …).
    pub short: &'static str,
    /// crates.io crate name (for `cargo install` fallback).
    pub krate: &'static str,
    /// Binary name on disk (may differ from crate name).
    pub bin: &'static str,
    /// Which verb installs it.
    pub kind: Kind,
}

const TABLE: &[Entry] = &[
    Entry {
        short: "daemon",
        krate: "crabtalkd",
        bin: "crabtalkd",
        kind: Kind::App,
    },
    Entry {
        short: "cli",
        krate: "crabtalk-cli",
        bin: "crabtalk",
        kind: Kind::App,
    },
];

impl Entry {
    /// All known registry entries.
    pub fn all() -> &'static [Self] {
        TABLE
    }

    /// Look up a table entry by short name.
    pub fn by_short(short: &str) -> Option<&'static Self> {
        TABLE.iter().find(|e| e.short == short)
    }

    /// Short names of one kind, for help text and errors.
    pub fn shorts_of(kind: Kind) -> Vec<&'static str> {
        TABLE
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.short)
            .collect()
    }

    /// Resolve a short name to its crates.io crate name. Unknown names pass through.
    pub fn resolve(name: &str) -> &str {
        Self::by_short(name).map(|e| e.krate).unwrap_or(name)
    }

    /// True if `krate` is a crabtalk-owned crate name.
    pub fn is_crabtalk(krate: &str) -> bool {
        krate == "crabtalkd" || krate.starts_with("crabtalk-") || krate == "crabup"
    }
}

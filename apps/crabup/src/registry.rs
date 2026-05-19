//! Short-name → crate/binary resolution + service metadata.

use std::path::PathBuf;

/// A first-party crabtalk binary that crabup knows about.
pub struct Entry {
    /// Short name used on the crabup CLI (`daemon`, `cli`, …).
    pub short: &'static str,
    /// crates.io crate name (for `cargo install` fallback).
    pub krate: &'static str,
    /// Binary name on disk (may differ from crate name).
    pub bin: &'static str,
    /// Reverse-DNS label for platform service unit, or `None` if non-serviceable.
    pub label: Option<&'static str>,
    /// Human description embedded in the unit file.
    pub description: &'static str,
}

const TABLE: &[Entry] = &[
    Entry {
        short: "daemon",
        krate: "crabtalkd",
        bin: "crabtalkd",
        label: Some("ai.crabtalk.daemon"),
        description: "Crabtalk daemon",
    },
    Entry {
        short: "cli",
        krate: "crabtalk-cli",
        bin: "crabtalk",
        label: None,
        description: "Crabtalk CLI client",
    },
    Entry {
        short: "telegram",
        krate: "crabtalk-telegram",
        bin: "crabtalk-telegram",
        label: Some("ai.crabtalk.telegram"),
        description: "Telegram gateway for Crabtalk",
    },
    Entry {
        short: "wechat",
        krate: "crabtalk-wechat",
        bin: "crabtalk-wechat",
        label: Some("ai.crabtalk.wechat"),
        description: "WeChat gateway for Crabtalk",
    },
    Entry {
        short: "search",
        krate: "crabtalk-search",
        bin: "crabtalk-search",
        label: Some("ai.crabtalk.search"),
        description: "Meta-search engine for Crabtalk",
    },
    Entry {
        short: "cron",
        krate: "crabtalk-cron",
        bin: "crabtalk-cron",
        label: Some("ai.crabtalk.cron"),
        description: "Cron scheduler for Crabtalk",
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

    /// Resolve a short name to its crates.io crate name. Unknown names pass through.
    pub fn resolve(name: &str) -> &str {
        Self::by_short(name).map(|e| e.krate).unwrap_or(name)
    }

    /// True if `krate` is a crabtalk-owned crate name.
    pub fn is_crabtalk(krate: &str) -> bool {
        krate == "crabtalkd" || krate.starts_with("crabtalk-") || krate == "crabup"
    }

    /// Locate this binary on disk.
    ///
    /// Search order: managed dir (`~/.crabtalk/bin/`), cargo dir
    /// (`~/.cargo/bin/`), then PATH.
    pub fn binary_path(&self) -> Option<PathBuf> {
        let managed = wcore::paths::BIN_DIR.join(self.bin);
        if managed.exists() {
            return Some(managed);
        }

        if let Some(home) = dirs::home_dir() {
            let cargo = home.join(".cargo/bin").join(self.bin);
            if cargo.exists() {
                return Some(cargo);
            }
        }

        let path = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(self.bin);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }
}

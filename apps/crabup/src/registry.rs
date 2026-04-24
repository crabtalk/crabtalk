//! Short-name → crates.io crate resolution + service metadata.

use std::path::PathBuf;

/// A first-party crabtalk binary that crabup knows about.
pub struct Entry {
    /// Short name used on the crabup CLI (`daemon`, `tui`, …).
    pub short: &'static str,
    /// crates.io crate name and binary name (they match for our binaries).
    pub krate: &'static str,
    /// Reverse-DNS label for platform service unit, or `None` if non-serviceable.
    pub label: Option<&'static str>,
    /// Human description embedded in the unit file.
    pub description: &'static str,
}

pub const DAEMON: Entry = Entry {
    short: "daemon",
    krate: "crabtalkd",
    label: Some("ai.crabtalk.daemon"),
    description: "Crabtalk daemon",
};

pub const TUI: Entry = Entry {
    short: "tui",
    krate: "crabtalk-tui",
    label: None,
    description: "Crabtalk TUI client",
};

pub const TELEGRAM: Entry = Entry {
    short: "telegram",
    krate: "crabtalk-telegram",
    label: Some("ai.crabtalk.telegram"),
    description: "Telegram gateway for Crabtalk",
};

pub const WECHAT: Entry = Entry {
    short: "wechat",
    krate: "crabtalk-wechat",
    label: Some("ai.crabtalk.wechat"),
    description: "WeChat gateway for Crabtalk",
};

pub const SEARCH: Entry = Entry {
    short: "search",
    krate: "crabtalk-search",
    label: Some("ai.crabtalk.search"),
    description: "Meta-search engine for Crabtalk",
};

pub const OUTLOOK: Entry = Entry {
    short: "outlook",
    krate: "crabtalk-outlook",
    label: Some("ai.crabtalk.outlook"),
    description: "Outlook integration for Crabtalk",
};

pub const CRON: Entry = Entry {
    short: "cron",
    krate: "crabtalk-cron",
    label: Some("ai.crabtalk.cron"),
    description: "Cron scheduler for Crabtalk",
};

const TABLE: &[&Entry] = &[&DAEMON, &TUI, &TELEGRAM, &WECHAT, &SEARCH, &OUTLOOK, &CRON];

impl Entry {
    /// Look up a table entry by short name.
    pub fn by_short(short: &str) -> Option<&'static Self> {
        TABLE.iter().find(|e| e.short == short).copied()
    }

    /// Resolve a short name to its crates.io crate name. Unknown names pass through.
    pub fn resolve(name: &str) -> &str {
        Self::by_short(name).map(|e| e.krate).unwrap_or(name)
    }

    /// True if `krate` is a crabtalk-owned crate name.
    pub fn is_crabtalk(krate: &str) -> bool {
        krate == "crabtalkd" || krate.starts_with("crabtalk-") || krate == "crabup"
    }

    /// Locate this binary on disk, preferring `~/.cargo/bin` where `cargo install` lands.
    pub fn binary_path(&self) -> Option<PathBuf> {
        if let Some(home) = dirs::home_dir() {
            let candidate = home.join(".cargo/bin").join(self.krate);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(self.krate);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }
}

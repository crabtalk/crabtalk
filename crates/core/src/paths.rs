//! Global paths for the crabtalk runtime.
//!
//! All crates resolve configuration, socket, and data paths through these
//! constants so there is a single source of truth.

use std::path::PathBuf;
use std::sync::LazyLock;

/// Global configuration directory (`~/.crabtalk/`).
pub static CONFIG_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    dirs::home_dir()
        .expect("no home directory")
        .join(".crabtalk")
});

/// Pinned socket path (`~/.crabtalk/crab.sock`).
pub static SOCKET_PATH: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("crab.sock"));

/// TCP port file (`~/.crabtalk/crab.tcp`). Contains the port number as text.
pub static TCP_PORT_FILE: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("crab.tcp"));

/// Logs directory (`~/.crabtalk/logs/`).
pub static LOGS_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("logs"));

/// Fallback working directory for the system service (`~/.crabtalk/home/`).
/// Agent commands use the daemon's actual cwd instead.
pub static HOME_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("home"));

/// Agents subdirectory (contains *.md files).
pub const AGENTS_DIR: &str = "agents";
/// Skills subdirectory.
pub const SKILLS_DIR: &str = "skills";
/// Data subdirectory.
pub const DATA_DIR: &str = "data";

/// SQLite memory database filename.
pub const MEMORY_DB: &str = "memory.db";

/// Default agent name used when no custom agents are configured.
pub const DEFAULT_AGENT: &str = "crab";

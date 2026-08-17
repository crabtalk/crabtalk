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

/// Runtime directory (`~/.crabtalk/run/`).
pub static RUN_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("run"));

/// Pinned socket path (`~/.crabtalk/run/crabtalk.sock`).
#[cfg(unix)]
pub static SOCKET_PATH: LazyLock<PathBuf> = LazyLock::new(|| RUN_DIR.join("crabtalk.sock"));

/// TCP port file (`~/.crabtalk/run/crabtalk.port`). Contains the port number as text.
pub static TCP_PORT_FILE: LazyLock<PathBuf> = LazyLock::new(|| RUN_DIR.join("crabtalk.port"));

/// Configuration file name.
pub const CONFIG_FILE: &str = "config.toml";
/// Mutable settings file (daemon-owned, persisted under `local/`).
pub const SETTINGS_FILE: &str = "local/settings.toml";
/// Daemon-owned state directory.
pub const LOCAL_DIR: &str = "local";
/// Skills subdirectory.
pub const SKILLS_DIR: &str = "local/skills";

/// Managed binary directory (`~/.crabtalk/bin/`).
pub static BIN_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("bin"));

/// Harness images (`~/.crabtalk/harnesses/`).
pub static HARNESSES_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("harnesses"));

/// OAuth token storage directory (`~/.crabtalk/tokens/`).
pub static TOKENS_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("tokens"));

/// Default agent name used when no custom agents are configured.
pub const DEFAULT_AGENT: &str = "crab";

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

/// Logs directory (`~/.crabtalk/logs/`).
pub static LOGS_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("logs"));

/// Configuration file name.
pub const CONFIG_FILE: &str = "config.toml";
/// Local package directory (user's own skills, agents, MCPs).
pub const LOCAL_DIR: &str = "local";
/// Hub-installed package manifests directory.
pub const PACKAGES_DIR: &str = "packages";
/// Agents subdirectory (contains *.md files).
pub const AGENTS_DIR: &str = "local/agents";
/// Skills subdirectory.
pub const SKILLS_DIR: &str = "local/skills";
/// Persisted conversation history directory (`~/.crabtalk/sessions/`).
pub static CONVERSATIONS_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("sessions"));

/// OAuth token storage directory (`~/.crabtalk/tokens/`).
pub static TOKENS_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("tokens"));

/// Default agent name used when no custom agents are configured.
pub const DEFAULT_AGENT: &str = "crab";

/// Port file for a named service: `~/.crabtalk/run/{name}.port`
pub fn service_port_file(name: &str) -> PathBuf {
    RUN_DIR.join(format!("{name}.port"))
}

/// Log file for a named service: `~/.crabtalk/logs/{name}.log`
pub fn service_log_path(name: &str) -> PathBuf {
    LOGS_DIR.join(format!("{name}.log"))
}

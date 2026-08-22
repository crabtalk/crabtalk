//! Where a crabtalk install lives on this machine.
//!
//! crabup creates these, the way rustup creates `~/.rustup`, so the installer
//! is where they are defined. Everything else reads them from here rather
//! than re-deriving the layout and drifting from it.

use std::{path::PathBuf, sync::LazyLock};

/// Environment variable naming the install root.
pub const HOME_VAR: &str = "CRABTALK_HOME";

/// Install root — `$CRABTALK_HOME`, else `~/.crabtalk/`.
pub static CONFIG_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var_os(HOME_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("no home directory")
                .join(".crabtalk")
        })
});

/// Managed binary directory (`~/.crabtalk/bin/`).
pub static BIN_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("bin"));

/// Runtime directory (`~/.crabtalk/run/`) — where a listener advertises itself
/// so a client can find it without being told.
pub static RUN_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("run"));

/// Pinned socket path (`~/.crabtalk/run/crabtalk.sock`).
pub static SOCKET_PATH: LazyLock<PathBuf> = LazyLock::new(|| RUN_DIR.join("crabtalk.sock"));

/// TCP port file (`~/.crabtalk/run/crabtalk.port`). Contains the port as text.
pub static PORT_FILE: LazyLock<PathBuf> = LazyLock::new(|| RUN_DIR.join("crabtalk.port"));

/// Top-level configuration (`~/.crabtalk/config.toml`).
pub static CONFIG_FILE: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("config.toml"));

/// Harness images (`~/.crabtalk/harnesses/`), one `{name}.elf` each.
pub static HARNESSES_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("harnesses"));

/// Cache root (`~/.crabtalk/cache/`), one subdirectory per thing that caches.
pub static CACHE_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("cache"));

/// The store file (`~/.crabtalk/store.crmem`).
pub static STORE_FILE: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("store.crmem"));

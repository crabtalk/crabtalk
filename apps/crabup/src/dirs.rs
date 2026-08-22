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

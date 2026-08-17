//! Where a crabtalk install lives on this machine.
//!
//! crabup creates these, the way rustup creates `~/.rustup`, so the installer
//! is where they are defined. Everything else reads them from here rather
//! than re-deriving the layout and drifting from it.

use std::{path::PathBuf, sync::LazyLock};

/// Install root (`~/.crabtalk/`).
pub static CONFIG_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    dirs::home_dir()
        .expect("no home directory")
        .join(".crabtalk")
});

/// Managed binary directory (`~/.crabtalk/bin/`).
pub static BIN_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONFIG_DIR.join("bin"));

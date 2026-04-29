//! Filesystem-backed persistence.
//!
//! [`FsStorage`] implements [`Storage`](wcore::storage::Storage)
//! with TOML configs, markdown prompts, and JSON session files.

pub use self::fs::{FsStorage, default_crab};
pub use loader::{DEFAULT_CONFIG, DEFAULT_SETTINGS, scaffold_config_dir};

pub(crate) mod fs;
mod loader;

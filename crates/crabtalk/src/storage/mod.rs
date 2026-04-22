//! Filesystem-backed persistence.
//!
//! [`FsStorage`] implements [`Storage`](wcore::storage::Storage)
//! with TOML configs, markdown prompts, and JSON session files.

pub use self::fs::{FsStorage, default_crab};
pub use loader::{DEFAULT_CONFIG, scaffold_config_dir};

mod fs;
mod loader;

//! Daemon config (`config.toml`) load/save.

use super::{FsStorage, atomic_write};
use anyhow::Result;
use wcore::DaemonConfig;

pub(super) fn load_config(storage: &FsStorage) -> Result<DaemonConfig> {
    let path = storage.config_dir.join(wcore::paths::CONFIG_FILE);
    if !path.exists() {
        return Ok(DaemonConfig::default());
    }
    DaemonConfig::load(&path)
}

pub(super) fn save_config(storage: &FsStorage, config: &DaemonConfig) -> Result<()> {
    let path = storage.config_dir.join(wcore::paths::CONFIG_FILE);
    let content = toml::to_string_pretty(config)?;
    atomic_write(&path, content.as_bytes())
}

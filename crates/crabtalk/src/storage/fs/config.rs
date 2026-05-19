//! Config (`config.toml`) load/save.

use super::{FsStorage, atomic_write};
use anyhow::Result;
use tokio::fs;
use wcore::Config;

pub(super) async fn load_config(storage: &FsStorage) -> Result<Config> {
    let path = storage.config_dir.join(wcore::paths::CONFIG_FILE);
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = fs::read_to_string(&path).await?;
    Config::from_toml(&content)
}

pub(super) async fn save_config(storage: &FsStorage, config: &Config) -> Result<()> {
    let path = storage.config_dir.join(wcore::paths::CONFIG_FILE);
    let content = toml::to_string_pretty(config)?;
    atomic_write(&path, content.as_bytes()).await
}

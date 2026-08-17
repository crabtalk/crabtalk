//! Config (`config.toml`) load/save.

use crate::Config;
use crate::backend::fs::{FsStorage, atomic_write};
use anyhow::Result;
use tokio::fs;

impl FsStorage {
    pub(super) async fn load_config(&self) -> Result<Config> {
        let path = self.config_dir.join(crate::CONFIG_FILE);
        if !path.exists() {
            return Ok(Config::default());
        }
        let content = fs::read_to_string(&path).await?;
        Config::from_toml(&content)
    }

    pub(super) async fn save_config(&self, config: &Config) -> Result<()> {
        let path = self.config_dir.join(crate::CONFIG_FILE);
        let content = toml::to_string_pretty(config)?;
        atomic_write(&path, content.as_bytes()).await
    }
}

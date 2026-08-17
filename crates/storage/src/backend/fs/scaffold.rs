//! First-startup scaffold: create the directory layout, write the config
//! templates, and seed the built-in `crab` agent.

use crate::AgentConfig;
use crate::backend::fs::FsStorage;
use anyhow::Result;
use tokio::fs;

/// Default template for `config.toml` — the hand-edited install config
/// (LLM endpoint, task pool, env vars).
pub const DEFAULT_CONFIG: &str = include_str!("../../../config.toml");

/// Default template for `local/settings.toml` — daemon-managed runtime
/// records (MCPs, agents). Overwritten on first daemon write.
pub const DEFAULT_SETTINGS: &str = include_str!("../../../settings.toml");

impl FsStorage {
    pub(super) async fn scaffold(&self, default_model: &str) -> Result<()> {
        fs::create_dir_all(&self.config_dir).await?;
        fs::create_dir_all(self.config_dir.join(crate::LOCAL_DIR)).await?;
        fs::create_dir_all(self.config_dir.join(crate::SKILLS_DIR)).await?;
        fs::create_dir_all(&self.sessions_root).await?;

        write_if_absent(&self.config_dir.join(crate::CONFIG_FILE), DEFAULT_CONFIG).await?;
        write_if_absent(
            &self.config_dir.join(crate::SETTINGS_FILE),
            DEFAULT_SETTINGS,
        )
        .await?;

        let file = self.read_settings().await?;
        if file.agents.is_empty() {
            self.upsert_agent(&AgentConfig::crab(default_model)).await?;
        }
        Ok(())
    }
}

async fn write_if_absent(path: &std::path::Path, contents: &str) -> Result<()> {
    if fs::try_exists(path).await? {
        return Ok(());
    }
    fs::write(path, contents).await?;
    Ok(())
}

//! First-startup scaffold: create the directory layout and seed the
//! built-in `crab` agent.

use crate::fs::FsStorage;
use anyhow::Result;
use tokio::fs;
use wcore::AgentConfig;

impl FsStorage {
    pub(super) async fn scaffold(&self, default_model: &str) -> Result<()> {
        fs::create_dir_all(&self.config_dir).await?;
        fs::create_dir_all(self.config_dir.join(wcore::paths::LOCAL_DIR)).await?;
        fs::create_dir_all(self.config_dir.join(wcore::paths::SKILLS_DIR)).await?;
        fs::create_dir_all(self.config_dir.join(wcore::paths::AGENTS_DIR)).await?;
        fs::create_dir_all(&self.sessions_root).await?;

        let file = self.read_settings().await?;
        if file.agents.is_empty() {
            self.upsert_agent(&AgentConfig::crab(default_model)).await?;
        }
        Ok(())
    }
}

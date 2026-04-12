//! Telegram bot configuration.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Telegram bot configuration.
///
/// Loaded from `~/.crabtalk/config/telegram.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub token: String,
    /// Optional whitelist of Telegram user IDs.
    ///
    /// When non-empty only messages from these users are processed;
    /// everyone else is silently ignored. When empty or omitted the
    /// bot responds to all users.
    #[serde(default)]
    pub allowed_users: Vec<i64>,
}

impl TelegramConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("invalid TOML in {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("failed to serialize TelegramConfig")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
    }
}

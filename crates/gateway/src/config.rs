//! Gateway configuration types.

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

/// WeChat bot configuration.
///
/// Loaded from `~/.crabtalk/config/wechat.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WechatConfig {
    /// Bot token from QR code login.
    pub token: String,
    /// API base URL (default: `https://ilinkai.weixin.qq.com`).
    #[serde(default = "WechatConfig::default_base_url")]
    pub base_url: String,
    /// Optional whitelist of WeChat user IDs (e.g. `xxx@im.wechat`).
    ///
    /// When non-empty only messages from these users are processed;
    /// everyone else is silently ignored.
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

impl WechatConfig {
    fn default_base_url() -> String {
        "https://ilinkai.weixin.qq.com".to_string()
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("invalid TOML in {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("failed to serialize WechatConfig")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
    }
}

//! Gateway configuration types.

use serde::{Deserialize, Serialize};

/// Telegram bot configuration.
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

/// Top-level gateway configuration.
///
/// Deserialized from `[gateway.telegram]` TOML tables.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    /// Telegram bot config. Absent means no Telegram bot.
    pub telegram: Option<TelegramConfig>,
}

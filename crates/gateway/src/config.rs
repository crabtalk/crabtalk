//! Gateway configuration types.

use serde::{Deserialize, Serialize};

/// Telegram bot configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub token: String,
}

/// Discord bot configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Bot token from the Discord developer portal.
    pub token: String,
}

/// Top-level gateway configuration.
///
/// Deserialized from `[gateway.telegram]` / `[gateway.discord]` TOML tables.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    /// Telegram bot config. Absent means no Telegram bot.
    pub telegram: Option<TelegramConfig>,
    /// Discord bot config. Absent means no Discord bot.
    pub discord: Option<DiscordConfig>,
}

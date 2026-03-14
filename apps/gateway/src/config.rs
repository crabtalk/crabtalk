//! Gateway configuration types.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported gateway platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GatewayType {
    /// Telegram bot via long-polling.
    Telegram,
    /// Discord bot via WebSocket gateway.
    Discord,
}

impl GatewayType {
    /// All known variants, in definition order.
    pub const VARIANTS: &[Self] = &[Self::Telegram, Self::Discord];

    /// URL hint for obtaining a bot token for this platform.
    pub fn token_hint(self) -> &'static str {
        match self {
            Self::Telegram => "https://core.telegram.org/bots#botfather",
            Self::Discord => "https://discord.com/developers/applications",
        }
    }
}

impl fmt::Display for GatewayType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Telegram => f.write_str("Telegram"),
            Self::Discord => f.write_str("Discord"),
        }
    }
}

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

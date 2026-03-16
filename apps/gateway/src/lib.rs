//! Walrus gateway — messaging platform integration for OpenWalrus agents.
//!
//! Provides configuration types and a spawn function that connects
//! platform bots (Telegram, Discord) to the daemon's agent event loop.

#[cfg(feature = "serve")]
pub mod client;
#[cfg(feature = "cli")]
pub mod cmd;
#[cfg(any(feature = "telegram", feature = "discord"))]
pub(crate) mod command;
pub mod config;
#[cfg(feature = "discord")]
pub(crate) mod discord;
pub mod message;
#[cfg(feature = "serve")]
pub mod spawn;
#[cfg(any(feature = "telegram", feature = "discord"))]
pub(crate) mod stream;
#[cfg(feature = "telegram")]
pub(crate) mod telegram;

#[cfg(feature = "discord")]
pub use config::DiscordConfig;
pub use config::GatewayConfig;
#[cfg(feature = "telegram")]
pub use config::TelegramConfig;
pub use message::{Attachment, AttachmentKind, GatewayMessage};

//! Walrus channel — messaging platform integration for OpenWalrus agents.
//!
//! Provides configuration types and a spawn function that connects
//! platform bots (Telegram, Discord) to the daemon's agent event loop.

pub(crate) mod command;
pub mod config;
pub(crate) mod discord;
pub mod message;
pub mod spawn;
pub(crate) mod telegram;

pub use config::{ChannelConfig, ChannelType, DiscordConfig, TelegramConfig};
pub use message::{Attachment, AttachmentKind, ChannelMessage};
pub use spawn::spawn_channels;

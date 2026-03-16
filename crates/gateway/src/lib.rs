//! Shared gateway types for OpenWalrus platform adapters.
//!
//! Provides configuration, message types, daemon client, stream accumulation,
//! and bot command parsing used by both the Telegram and Discord binaries.

use compact_str::CompactString;
use std::{collections::HashSet, path::Path, sync::Arc};
use tokio::sync::RwLock;

pub mod client;
pub mod command;
pub mod config;
pub mod message;
pub mod stream;

pub use client::DaemonClient;
pub use command::{BotCommand, COMMAND_HINT, parse_command};
pub use config::{DiscordConfig, GatewayConfig, TelegramConfig};
pub use message::{Attachment, AttachmentKind, GatewayMessage, attachment_summary};
pub use stream::StreamAccumulator;

/// Shared set of sender IDs belonging to sibling Walrus bots.
///
/// Built incrementally as each bot connects. Channel loops check this set
/// before dispatching messages — senders in this set are silently dropped
/// to prevent agent-to-agent loops.
pub type KnownBots = Arc<RwLock<HashSet<CompactString>>>;

/// Result of a streaming request to the daemon.
pub enum StreamResult {
    Ok { session_id: u64 },
    SessionError,
    Failed,
}

/// Read the agents directory and return the first agent name found,
/// falling back to "assistant".
pub fn resolve_default_agent(agents_dir: &Path) -> CompactString {
    let Ok(entries) = std::fs::read_dir(agents_dir) else {
        return CompactString::from("assistant");
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            return CompactString::from(stem);
        }
    }
    CompactString::from("assistant")
}

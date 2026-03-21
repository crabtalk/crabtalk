//! Telegram bot command dispatch.
//!
//! Executes parsed bot commands (hub install/uninstall) by calling
//! crabhub library functions directly and streaming progress back
//! to the originating Telegram chat.

use crate::BotCommand;
use teloxide::prelude::*;

/// Execute a bot command, streaming progress messages back to the originating chat.
pub async fn dispatch_command(cmd: BotCommand, bot: Bot, chat_id: i64) {
    let (package, is_install) = match cmd {
        BotCommand::HubInstall { package } => (package, true),
        BotCommand::HubUninstall { package } => (package, false),
    };

    send_text(&bot, chat_id, format!("Starting: {package}...")).await;
    let on_step = |msg: &str| {
        tracing::info!("hub: {msg}");
    };

    let result = if is_install {
        crabhub::package::install(&package, &[], on_step).await
    } else {
        crabhub::package::uninstall(&package, &[], on_step).await
    };

    match result {
        Ok(()) => send_text(&bot, chat_id, format!("Done: {package}")).await,
        Err(e) => send_text(&bot, chat_id, format!("Failed: {e}")).await,
    }
}

async fn send_text(bot: &Bot, chat_id: i64, content: String) {
    if let Err(e) = bot.send_message(ChatId(chat_id), content).await {
        tracing::warn!("failed to send bot command reply: {e}");
    }
}

//! Telegram bot command dispatch.
//!
//! Executes parsed bot commands (hub install/uninstall) by streaming
//! progress back to the originating Telegram chat.

use crate::command::BotCommand;
use compact_str::CompactString;
use std::{future::Future, sync::Arc};
use teloxide::prelude::*;
use tokio::sync::mpsc;
use wcore::protocol::message::{
    client::{ClientMessage, HubAction},
    server::{DownloadEvent, ServerMessage},
};

/// Execute a bot command, streaming progress messages back to the originating chat.
pub(crate) async fn dispatch_command<C, CFut>(
    cmd: BotCommand,
    on_message: Arc<C>,
    bot: Bot,
    chat_id: i64,
) where
    C: Fn(ClientMessage) -> CFut + Send + Sync + 'static,
    CFut: Future<Output = mpsc::UnboundedReceiver<ServerMessage>> + Send + 'static,
{
    let msg = match cmd {
        BotCommand::HubInstall { package } => ClientMessage::Hub {
            package: CompactString::from(&package),
            action: HubAction::Install,
        },
        BotCommand::HubUninstall { package } => ClientMessage::Hub {
            package: CompactString::from(&package),
            action: HubAction::Uninstall,
        },
    };

    let mut rx = on_message(msg).await;
    while let Some(server_msg) = rx.recv().await {
        match server_msg {
            ServerMessage::Download(event) => match event {
                DownloadEvent::Created { label, .. } => {
                    send_text(&bot, chat_id, format!("Starting: {label}...")).await;
                }
                DownloadEvent::Step { message, .. } => {
                    send_text(&bot, chat_id, format!("  {message}")).await;
                }
                DownloadEvent::Progress { .. } => {}
                DownloadEvent::Completed { .. } => {
                    send_text(&bot, chat_id, "Done".to_string()).await;
                }
                DownloadEvent::Failed { error, .. } => {
                    send_text(&bot, chat_id, format!("Failed: {error}")).await;
                }
            },
            ServerMessage::Error { message, .. } => {
                tracing::warn!("command error: {message}");
            }
            _ => {}
        }
    }
}

/// Send a plain-text message to the chat.
async fn send_text(bot: &Bot, chat_id: i64, content: String) {
    if let Err(e) = bot.send_message(ChatId(chat_id), content).await {
        tracing::warn!("failed to send bot command reply: {e}");
    }
}

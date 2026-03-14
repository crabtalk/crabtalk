//! Telegram bot command dispatch.
//!
//! Executes parsed bot commands (hub install/uninstall) by streaming
//! progress back to the originating Telegram chat.

use crate::{client::DaemonClient, command::BotCommand};
use std::sync::Arc;
use teloxide::prelude::*;
use wcore::protocol::message::{
    ClientMessage, DownloadCreated, DownloadStep, HubAction, HubMsg, ServerMessage, client_message,
    download_event, server_message,
};

/// Execute a bot command, streaming progress messages back to the originating chat.
pub(crate) async fn dispatch_command(
    cmd: BotCommand,
    client: Arc<DaemonClient>,
    bot: Bot,
    chat_id: i64,
) {
    let msg = match cmd {
        BotCommand::HubInstall { package } => ClientMessage {
            msg: Some(client_message::Msg::Hub(HubMsg {
                package,
                action: HubAction::Install as i32,
            })),
        },
        BotCommand::HubUninstall { package } => ClientMessage {
            msg: Some(client_message::Msg::Hub(HubMsg {
                package,
                action: HubAction::Uninstall as i32,
            })),
        },
    };

    let mut rx = client.send(msg).await;
    while let Some(server_msg) = rx.recv().await {
        match server_msg {
            ServerMessage {
                msg: Some(server_message::Msg::Download(event)),
            } => match event.event {
                Some(download_event::Event::Created(DownloadCreated { label, .. })) => {
                    send_text(&bot, chat_id, format!("Starting: {label}...")).await;
                }
                Some(download_event::Event::Step(DownloadStep { message, .. })) => {
                    send_text(&bot, chat_id, format!("  {message}")).await;
                }
                Some(download_event::Event::Progress(_)) => {}
                Some(download_event::Event::Completed(_)) => {
                    send_text(&bot, chat_id, "Done".to_string()).await;
                }
                Some(download_event::Event::Failed(f)) => {
                    send_text(&bot, chat_id, format!("Failed: {}", f.error)).await;
                }
                None => {}
            },
            ServerMessage {
                msg: Some(server_message::Msg::Error(err)),
            } => {
                tracing::warn!("command error: {}", err.message);
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

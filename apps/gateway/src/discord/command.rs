//! Discord bot command dispatch.
//!
//! Executes parsed bot commands (hub install/uninstall) by streaming
//! progress back to the originating Discord channel.

use crate::{client::DaemonClient, command::BotCommand};
use serenity::model::id::ChannelId;
use std::sync::Arc;
use wcore::protocol::message::{
    ClientMessage, DownloadCreated, DownloadStep, HubAction, HubMsg, ServerMessage, client_message,
    download_event, server_message,
};

/// Execute a bot command, streaming progress messages back to the originating channel.
pub(crate) async fn dispatch_command(
    cmd: BotCommand,
    client: Arc<DaemonClient>,
    http: Arc<serenity::http::Http>,
    channel_id: ChannelId,
) {
    let msg = match cmd {
        BotCommand::HubInstall { package } => ClientMessage {
            msg: Some(client_message::Msg::Hub(HubMsg {
                package,
                action: HubAction::Install as i32,
                filters: vec![],
            })),
        },
        BotCommand::HubUninstall { package } => ClientMessage {
            msg: Some(client_message::Msg::Hub(HubMsg {
                package,
                action: HubAction::Uninstall as i32,
                filters: vec![],
            })),
        },
        BotCommand::Switch { .. } => return, // Handled inline in the main loop.
    };

    let mut rx = client.send(msg).await;
    while let Some(server_msg) = rx.recv().await {
        match server_msg {
            ServerMessage {
                msg: Some(server_message::Msg::Download(event)),
            } => match event.event {
                Some(download_event::Event::Created(DownloadCreated { label, .. })) => {
                    send_text(&http, channel_id, format!("Starting: {label}...")).await;
                }
                Some(download_event::Event::Step(DownloadStep { message, .. })) => {
                    send_text(&http, channel_id, format!("  {message}")).await;
                }
                Some(download_event::Event::Progress(_)) => {}
                Some(download_event::Event::Completed(_)) => {
                    send_text(&http, channel_id, "Done".to_string()).await;
                }
                Some(download_event::Event::Failed(f)) => {
                    send_text(&http, channel_id, format!("Failed: {}", f.error)).await;
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

/// Send a plain-text message to the channel.
async fn send_text(http: &Arc<serenity::http::Http>, channel_id: ChannelId, content: String) {
    if let Err(e) = channel_id.say(http, content).await {
        tracing::warn!("failed to send bot command reply: {e}");
    }
}

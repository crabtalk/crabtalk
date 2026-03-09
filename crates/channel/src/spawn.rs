//! Channel spawn logic.
//!
//! Connects configured platform bots (Telegram, Discord) and routes all
//! messages through a single `on_message` callback that accepts a
//! `ClientMessage` and returns a `ServerMessage` stream.

use crate::command::parse_command;
use crate::config::{ChannelConfig, DiscordConfig, TelegramConfig};
use crate::message::ChannelMessage;
use compact_str::CompactString;
use serenity::model::id::ChannelId;
use std::{collections::HashMap, future::Future, sync::Arc};
use teloxide::prelude::*;
use tokio::sync::mpsc;
use wcore::protocol::message::{client::ClientMessage, server::ServerMessage};

/// Connect configured channels and spawn message loops.
///
/// Spawns transports for each configured platform (Telegram, Discord).
/// `default_agent` is used when a platform config does not specify an agent.
/// `on_message` dispatches any `ClientMessage` and returns a receiver for
/// streamed `ServerMessage` results.
pub async fn spawn_channels<C, CFut>(
    config: &ChannelConfig,
    default_agent: CompactString,
    on_message: Arc<C>,
) where
    C: Fn(ClientMessage) -> CFut + Send + Sync + 'static,
    CFut: Future<Output = mpsc::UnboundedReceiver<ServerMessage>> + Send + 'static,
{
    if let Some(tg) = &config.telegram {
        spawn_telegram(tg, &default_agent, on_message.clone()).await;
    }

    if let Some(dc) = &config.discord {
        spawn_discord(dc, &default_agent, on_message.clone()).await;
    }
}

async fn spawn_telegram<C, CFut>(
    tg: &TelegramConfig,
    default_agent: &CompactString,
    on_message: Arc<C>,
) where
    C: Fn(ClientMessage) -> CFut + Send + Sync + 'static,
    CFut: Future<Output = mpsc::UnboundedReceiver<ServerMessage>> + Send + 'static,
{
    let agent = tg
        .agent
        .as_deref()
        .map(CompactString::from)
        .unwrap_or_else(|| default_agent.clone());

    let bot = Bot::new(&tg.bot);
    let (tx, rx) = mpsc::unbounded_channel::<ChannelMessage>();

    let poll_bot = bot.clone();
    tokio::spawn(async move {
        crate::telegram::poll_loop(poll_bot, tx).await;
    });

    tokio::spawn(telegram_loop(rx, bot, agent, on_message));
    tracing::info!(platform = "telegram", "channel transport started");
}

async fn spawn_discord<C, CFut>(
    dc: &DiscordConfig,
    default_agent: &CompactString,
    on_message: Arc<C>,
) where
    C: Fn(ClientMessage) -> CFut + Send + Sync + 'static,
    CFut: Future<Output = mpsc::UnboundedReceiver<ServerMessage>> + Send + 'static,
{
    let agent = dc
        .agent
        .as_deref()
        .map(CompactString::from)
        .unwrap_or_else(|| default_agent.clone());

    let (msg_tx, msg_rx) = mpsc::unbounded_channel::<ChannelMessage>();
    let (http_tx, http_rx) = tokio::sync::oneshot::channel();

    let token = dc.token.clone();
    tokio::spawn(async move {
        crate::discord::event_loop(&token, msg_tx, http_tx).await;
    });

    tokio::spawn(async move {
        match http_rx.await {
            Ok(http) => {
                discord_loop(msg_rx, http, agent, on_message).await;
            }
            Err(_) => {
                tracing::error!("discord gateway failed to send http client");
            }
        }
    });

    tracing::info!(platform = "discord", "channel transport started");
}

/// Telegram message loop: routes incoming messages to agents or bot commands.
///
/// Maintains a `chat_id → session_id` mapping so consecutive messages from the
/// same chat reuse the same session. If a session is killed externally, the
/// error triggers a retry with `session: None` to create a fresh session.
async fn telegram_loop<C, CFut>(
    mut rx: mpsc::UnboundedReceiver<ChannelMessage>,
    bot: Bot,
    agent: CompactString,
    on_message: Arc<C>,
) where
    C: Fn(ClientMessage) -> CFut + Send + Sync + 'static,
    CFut: Future<Output = mpsc::UnboundedReceiver<ServerMessage>> + Send + 'static,
{
    let mut sessions: HashMap<i64, u64> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let content = msg.content.clone();

        tracing::info!(%agent, chat_id, "telegram dispatch");

        // Bot command path.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(cmd) => {
                    let b = bot.clone();
                    let om = on_message.clone();
                    tokio::spawn(async move {
                        crate::telegram::command::dispatch_command(cmd, om, b, chat_id).await;
                    });
                }
                None => {
                    tracing::warn!(chat_id, content, "unrecognised bot command");
                    let hint = "Unknown command. Available: /hub install <pkg>, /hub uninstall <pkg>, /model download <model>";
                    if let Err(e) = bot.send_message(ChatId(chat_id), hint).await {
                        tracing::warn!("failed to send command hint: {e}");
                    }
                }
            }
            continue;
        }

        // Normal agent chat path with session mapping.
        let session = sessions.get(&chat_id).copied();
        let client_msg = ClientMessage::Send {
            agent: agent.clone(),
            content: content.clone(),
            session,
        };
        let mut reply_rx = on_message(client_msg).await;
        let mut retry = false;
        while let Some(server_msg) = reply_rx.recv().await {
            match server_msg {
                ServerMessage::Response(resp) => {
                    sessions.insert(chat_id, resp.session);
                    if let Err(e) = bot.send_message(ChatId(chat_id), resp.content).await {
                        tracing::warn!(%agent, "failed to send channel reply: {e}");
                    }
                }
                ServerMessage::Error { ref message, .. } if session.is_some() => {
                    tracing::warn!(%agent, chat_id, "session error, retrying: {message}");
                    sessions.remove(&chat_id);
                    retry = true;
                }
                ServerMessage::Error { message, .. } => {
                    tracing::warn!(%agent, chat_id, "dispatch error: {message}");
                }
                _ => {}
            }
        }

        // Retry with a fresh session if the previous one was stale.
        if retry {
            let client_msg = ClientMessage::Send {
                agent: agent.clone(),
                content,
                session: None,
            };
            let mut reply_rx = on_message(client_msg).await;
            while let Some(server_msg) = reply_rx.recv().await {
                match server_msg {
                    ServerMessage::Response(resp) => {
                        sessions.insert(chat_id, resp.session);
                        if let Err(e) = bot.send_message(ChatId(chat_id), resp.content).await {
                            tracing::warn!(%agent, "failed to send channel reply: {e}");
                        }
                    }
                    ServerMessage::Error { message, .. } => {
                        tracing::warn!(%agent, chat_id, "dispatch error on retry: {message}");
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!(platform = "telegram", "channel loop ended");
}

/// Discord message loop: routes incoming messages to agents or bot commands.
///
/// Maintains a `chat_id → session_id` mapping so consecutive messages from the
/// same chat reuse the same session. Same stale-session retry logic as Telegram.
async fn discord_loop<C, CFut>(
    mut rx: mpsc::UnboundedReceiver<ChannelMessage>,
    http: Arc<serenity::http::Http>,
    agent: CompactString,
    on_message: Arc<C>,
) where
    C: Fn(ClientMessage) -> CFut + Send + Sync + 'static,
    CFut: Future<Output = mpsc::UnboundedReceiver<ServerMessage>> + Send + 'static,
{
    let mut sessions: HashMap<i64, u64> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let channel_id = ChannelId::new(chat_id as u64);
        let content = msg.content.clone();

        tracing::info!(%agent, chat_id, "discord dispatch");

        // Bot command path.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(cmd) => {
                    let h = http.clone();
                    let om = on_message.clone();
                    tokio::spawn(async move {
                        crate::discord::command::dispatch_command(cmd, om, h, channel_id).await;
                    });
                }
                None => {
                    tracing::warn!(chat_id, content, "unrecognised bot command");
                    let hint = "Unknown command. Available: /hub install <pkg>, /hub uninstall <pkg>, /model download <model>";
                    crate::discord::send_text(&http, channel_id, hint.to_owned()).await;
                }
            }
            continue;
        }

        // Normal agent chat path with session mapping.
        let session = sessions.get(&chat_id).copied();
        let client_msg = ClientMessage::Send {
            agent: agent.clone(),
            content: content.clone(),
            session,
        };
        let mut reply_rx = on_message(client_msg).await;
        let mut retry = false;
        while let Some(server_msg) = reply_rx.recv().await {
            match server_msg {
                ServerMessage::Response(resp) => {
                    sessions.insert(chat_id, resp.session);
                    crate::discord::send_text(&http, channel_id, resp.content).await;
                }
                ServerMessage::Error { ref message, .. } if session.is_some() => {
                    tracing::warn!(%agent, chat_id, "session error, retrying: {message}");
                    sessions.remove(&chat_id);
                    retry = true;
                }
                ServerMessage::Error { message, .. } => {
                    tracing::warn!(%agent, chat_id, "dispatch error: {message}");
                }
                _ => {}
            }
        }

        // Retry with a fresh session if the previous one was stale.
        if retry {
            let client_msg = ClientMessage::Send {
                agent: agent.clone(),
                content,
                session: None,
            };
            let mut reply_rx = on_message(client_msg).await;
            while let Some(server_msg) = reply_rx.recv().await {
                match server_msg {
                    ServerMessage::Response(resp) => {
                        sessions.insert(chat_id, resp.session);
                        crate::discord::send_text(&http, channel_id, resp.content).await;
                    }
                    ServerMessage::Error { message, .. } => {
                        tracing::warn!(%agent, chat_id, "dispatch error on retry: {message}");
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!(platform = "discord", "channel loop ended");
}

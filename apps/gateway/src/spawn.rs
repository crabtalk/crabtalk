//! Gateway spawn logic.
//!
//! Connects configured platform bots (Telegram, Discord) and routes all
//! messages through a `DaemonClient` that speaks the walrus protocol
//! over a UDS connection.

use crate::{client::DaemonClient, config::GatewayConfig};
#[cfg(any(feature = "telegram", feature = "discord"))]
use crate::{command::parse_command, message::GatewayMessage};
use compact_str::CompactString;
#[cfg(feature = "discord")]
use serenity::model::id::ChannelId;
#[cfg(any(feature = "telegram", feature = "discord"))]
use std::collections::HashMap;
use std::{collections::HashSet, sync::Arc};
#[cfg(feature = "telegram")]
use teloxide::prelude::*;
use tokio::sync::RwLock;
#[cfg(any(feature = "telegram", feature = "discord"))]
use tokio::sync::mpsc;
#[cfg(any(feature = "telegram", feature = "discord"))]
use wcore::protocol::message::{ClientMessage, SendMsg, ServerMessage, server_message};

/// Shared set of sender IDs belonging to sibling Walrus bots.
///
/// Built incrementally as each bot connects. Channel loops check this set
/// before dispatching messages — senders in this set are silently dropped
/// to prevent agent-to-agent loops.
type KnownBots = Arc<RwLock<HashSet<CompactString>>>;

/// Connect configured gateways and spawn message loops.
///
/// Iterates all gateway entries and spawns a transport for each one.
/// `default_agent` is used when an entry does not specify an agent.
#[allow(unused_variables)]
pub async fn spawn_gateways(
    config: &GatewayConfig,
    default_agent: CompactString,
    client: Arc<DaemonClient>,
) {
    let known_bots: KnownBots = Arc::new(RwLock::new(HashSet::new()));

    #[cfg(feature = "telegram")]
    if let Some(tg) = &config.telegram {
        if tg.token.is_empty() {
            tracing::warn!(platform = "telegram", "token is empty, skipping");
        } else {
            spawn_telegram(
                &tg.token,
                default_agent.clone(),
                client.clone(),
                known_bots.clone(),
            )
            .await;
        }
    }

    #[cfg(feature = "discord")]
    if let Some(dc) = &config.discord {
        if dc.token.is_empty() {
            tracing::warn!(platform = "discord", "token is empty, skipping");
        } else {
            spawn_discord(&dc.token, default_agent, client, known_bots).await;
        }
    }
}

#[cfg(feature = "telegram")]
async fn spawn_telegram(
    token: &str,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let bot = Bot::new(token);

    // Resolve our own user ID and register it in the known-bot set.
    match bot.get_me().await {
        Ok(me) => {
            let bot_sender: CompactString = format!("tg:{}", me.id.0).into();
            tracing::info!(platform = "telegram", %bot_sender, "registered bot identity");
            known_bots.write().await.insert(bot_sender);
        }
        Err(e) => {
            tracing::warn!(platform = "telegram", "failed to resolve bot identity: {e}");
        }
    }

    let (tx, rx) = mpsc::unbounded_channel::<GatewayMessage>();

    let poll_bot = bot.clone();
    tokio::spawn(async move {
        crate::telegram::poll_loop(poll_bot, tx).await;
    });

    tokio::spawn(telegram_loop(rx, bot, agent, client, known_bots));
    tracing::info!(platform = "telegram", "channel transport started");
}

#[cfg(feature = "discord")]
async fn spawn_discord(
    token: &str,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let (msg_tx, msg_rx) = mpsc::unbounded_channel::<GatewayMessage>();
    let (http_tx, http_rx) = tokio::sync::oneshot::channel();

    let token = token.to_owned();
    let kb = known_bots.clone();
    tokio::spawn(async move {
        crate::discord::event_loop(&token, msg_tx, http_tx, kb).await;
    });

    tokio::spawn(async move {
        match http_rx.await {
            Ok(http) => {
                discord_loop(msg_rx, http, agent, client, known_bots).await;
            }
            Err(_) => {
                tracing::error!("discord gateway failed to send http client");
            }
        }
    });

    tracing::info!(platform = "discord", "channel transport started");
}

#[cfg(feature = "telegram")]
/// Telegram message loop: routes incoming messages to agents or bot commands.
///
/// Maintains a `chat_id → session_id` mapping so consecutive messages from the
/// same chat reuse the same session. If a session is killed externally, the
/// error triggers a retry with `session: None` to create a fresh session.
async fn telegram_loop(
    mut rx: mpsc::UnboundedReceiver<GatewayMessage>,
    bot: Bot,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let mut sessions: HashMap<i64, u64> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let content = msg.content.clone();
        let sender: CompactString = format!("tg:{}", msg.sender_id).into();

        // Drop messages from sibling Walrus bots.
        if known_bots.read().await.contains(&sender) {
            tracing::debug!(%sender, chat_id, "dropping message from known bot");
            continue;
        }

        tracing::info!(%agent, chat_id, "telegram dispatch");

        // Bot command path.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(cmd) => {
                    let b = bot.clone();
                    let c = client.clone();
                    tokio::spawn(async move {
                        crate::telegram::command::dispatch_command(cmd, c, b, chat_id).await;
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

        let client_msg = ClientMessage::from(SendMsg {
            agent: agent.clone().into(),
            content: content.clone(),
            session,
            sender: Some(sender.to_string()),
        });
        let mut reply_rx = client.send(client_msg).await;
        let mut retry = false;
        while let Some(server_msg) = reply_rx.recv().await {
            match server_msg {
                ServerMessage {
                    msg: Some(server_message::Msg::Response(resp)),
                } => {
                    sessions.insert(chat_id, resp.session);
                    if let Err(e) = bot.send_message(ChatId(chat_id), resp.content).await {
                        tracing::warn!(%agent, "failed to send channel reply: {e}");
                    }
                }
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ref err)),
                } if session.is_some() => {
                    tracing::warn!(%agent, chat_id, "session error, retrying: {}", err.message);
                    sessions.remove(&chat_id);
                    retry = true;
                }
                ServerMessage {
                    msg: Some(server_message::Msg::Error(err)),
                } => {
                    tracing::warn!(%agent, chat_id, "dispatch error: {}", err.message);
                }
                _ => {}
            }
        }

        // Retry with a fresh session if the previous one was stale.
        if retry {
            let client_msg = ClientMessage::from(SendMsg {
                agent: agent.clone().into(),
                content,
                session: None,
                sender: Some(sender.to_string()),
            });
            let mut reply_rx = client.send(client_msg).await;
            while let Some(server_msg) = reply_rx.recv().await {
                match server_msg {
                    ServerMessage {
                        msg: Some(server_message::Msg::Response(resp)),
                    } => {
                        sessions.insert(chat_id, resp.session);
                        if let Err(e) = bot.send_message(ChatId(chat_id), resp.content).await {
                            tracing::warn!(%agent, "failed to send channel reply: {e}");
                        }
                    }
                    ServerMessage {
                        msg: Some(server_message::Msg::Error(err)),
                    } => {
                        tracing::warn!(%agent, chat_id, "dispatch error on retry: {}", err.message);
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!(platform = "telegram", "channel loop ended");
}

#[cfg(feature = "discord")]
/// Discord message loop: routes incoming messages to agents or bot commands.
///
/// Maintains a `chat_id → session_id` mapping so consecutive messages from the
/// same chat reuse the same session. Same stale-session retry logic as Telegram.
async fn discord_loop(
    mut rx: mpsc::UnboundedReceiver<GatewayMessage>,
    http: Arc<serenity::http::Http>,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let mut sessions: HashMap<i64, u64> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let channel_id = ChannelId::new(chat_id as u64);
        let content = msg.content.clone();
        let sender: CompactString = format!("dc:{}", msg.sender_id).into();

        // Drop messages from sibling Walrus bots.
        if known_bots.read().await.contains(&sender) {
            tracing::debug!(%sender, chat_id, "dropping message from known bot");
            continue;
        }

        tracing::info!(%agent, chat_id, "discord dispatch");

        // Bot command path.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(cmd) => {
                    let h = http.clone();
                    let c = client.clone();
                    tokio::spawn(async move {
                        crate::discord::command::dispatch_command(cmd, c, h, channel_id).await;
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

        let client_msg = ClientMessage::from(SendMsg {
            agent: agent.clone().into(),
            content: content.clone(),
            session,
            sender: Some(sender.to_string()),
        });
        let mut reply_rx = client.send(client_msg).await;
        let mut retry = false;
        while let Some(server_msg) = reply_rx.recv().await {
            match server_msg {
                ServerMessage {
                    msg: Some(server_message::Msg::Response(resp)),
                } => {
                    sessions.insert(chat_id, resp.session);
                    crate::discord::send_text(&http, channel_id, resp.content).await;
                }
                ServerMessage {
                    msg: Some(server_message::Msg::Error(ref err)),
                } if session.is_some() => {
                    tracing::warn!(%agent, chat_id, "session error, retrying: {}", err.message);
                    sessions.remove(&chat_id);
                    retry = true;
                }
                ServerMessage {
                    msg: Some(server_message::Msg::Error(err)),
                } => {
                    tracing::warn!(%agent, chat_id, "dispatch error: {}", err.message);
                }
                _ => {}
            }
        }

        // Retry with a fresh session if the previous one was stale.
        if retry {
            let client_msg = ClientMessage::from(SendMsg {
                agent: agent.clone().into(),
                content,
                session: None,
                sender: Some(sender.to_string()),
            });
            let mut reply_rx = client.send(client_msg).await;
            while let Some(server_msg) = reply_rx.recv().await {
                match server_msg {
                    ServerMessage {
                        msg: Some(server_message::Msg::Response(resp)),
                    } => {
                        sessions.insert(chat_id, resp.session);
                        crate::discord::send_text(&http, channel_id, resp.content).await;
                    }
                    ServerMessage {
                        msg: Some(server_message::Msg::Error(err)),
                    } => {
                        tracing::warn!(%agent, chat_id, "dispatch error on retry: {}", err.message);
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::info!(platform = "discord", "channel loop ended");
}

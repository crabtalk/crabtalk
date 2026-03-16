//! Gateway spawn logic.
//!
//! Connects configured platform bots (Telegram, Discord) and routes all
//! messages through a `DaemonClient` that speaks the walrus protocol
//! over a UDS connection.

use crate::{client::DaemonClient, config::GatewayConfig};
#[cfg(any(feature = "telegram", feature = "discord"))]
use crate::{command::parse_command, message::GatewayMessage, stream::StreamAccumulator};
use compact_str::CompactString;
#[cfg(feature = "discord")]
use serenity::model::id::ChannelId;
#[cfg(any(feature = "telegram", feature = "discord"))]
use std::collections::HashMap;
use std::{collections::HashSet, sync::Arc};
#[cfg(feature = "telegram")]
use teloxide::prelude::*;
#[cfg(feature = "telegram")]
use teloxide::types::ChatAction;
use tokio::sync::RwLock;
#[cfg(any(feature = "telegram", feature = "discord"))]
use tokio::sync::mpsc;
#[cfg(any(feature = "telegram", feature = "discord"))]
use wcore::protocol::message::{ClientMessage, ServerMessage, StreamMsg, server_message};

/// Shared set of sender IDs belonging to sibling Walrus bots.
///
/// Built incrementally as each bot connects. Channel loops check this set
/// before dispatching messages — senders in this set are silently dropped
/// to prevent agent-to-agent loops.
type KnownBots = Arc<RwLock<HashSet<CompactString>>>;

/// Result of a streaming request to the daemon.
#[cfg(any(feature = "telegram", feature = "discord"))]
enum StreamResult {
    Ok { session_id: u64 },
    SessionError,
    Failed,
}

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
/// same chat reuse the same session. Uses `StreamMsg` for streaming responses
/// with periodic message editing.
async fn telegram_loop(
    mut rx: mpsc::UnboundedReceiver<GatewayMessage>,
    bot: Bot,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let mut sessions: HashMap<i64, u64> = HashMap::new();
    let mut chat_agents: HashMap<i64, CompactString> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let content = msg.content.clone();
        let sender: CompactString = format!("tg:{}", msg.sender_id).into();

        // Drop messages from sibling Walrus bots.
        if known_bots.read().await.contains(&sender) {
            tracing::debug!(%sender, chat_id, "dropping message from known bot");
            continue;
        }

        let active_agent = chat_agents.get(&chat_id).unwrap_or(&agent);
        tracing::info!(agent = %active_agent, chat_id, "telegram dispatch");

        // Bot command path.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(crate::command::BotCommand::Switch { agent: new_agent }) => {
                    let new_agent: CompactString = new_agent.into();
                    chat_agents.insert(chat_id, new_agent.clone());
                    sessions.remove(&chat_id);
                    let msg = format!("Switched to agent: {new_agent}");
                    if let Err(e) = bot.send_message(ChatId(chat_id), msg).await {
                        tracing::warn!("failed to send switch confirmation: {e}");
                    }
                }
                Some(cmd) => {
                    let b = bot.clone();
                    let c = client.clone();
                    tokio::spawn(async move {
                        crate::telegram::command::dispatch_command(cmd, c, b, chat_id).await;
                    });
                }
                None => {
                    tracing::warn!(chat_id, content, "unrecognised bot command");
                    if let Err(e) = bot
                        .send_message(ChatId(chat_id), crate::command::COMMAND_HINT)
                        .await
                    {
                        tracing::warn!("failed to send command hint: {e}");
                    }
                }
            }
            continue;
        }

        // Normal agent chat path with session mapping.
        let session = sessions.get(&chat_id).copied();

        // Append attachment summary to content if present.
        let content = match crate::message::attachment_summary(&msg.attachments) {
            Some(summary) => format!("{content}\n{summary}"),
            None => content,
        };

        let result = tg_stream(
            &bot,
            &client,
            active_agent,
            chat_id,
            msg.message_id,
            &content,
            &sender,
            session,
        )
        .await;

        match result {
            StreamResult::Ok { session_id } => {
                sessions.insert(chat_id, session_id);
            }
            StreamResult::SessionError if session.is_some() => {
                // Stale session — retry with a fresh one.
                tracing::warn!(agent = %active_agent, chat_id, "session error, retrying");
                sessions.remove(&chat_id);
                let retry = tg_stream(
                    &bot,
                    &client,
                    active_agent,
                    chat_id,
                    msg.message_id,
                    &content,
                    &sender,
                    None,
                )
                .await;
                if let StreamResult::Ok { session_id } = retry {
                    sessions.insert(chat_id, session_id);
                }
            }
            StreamResult::SessionError | StreamResult::Failed => {}
        }
    }

    tracing::info!(platform = "telegram", "channel loop ended");
}

/// Send a streaming request to the daemon and edit a Telegram message as
/// chunks arrive. Returns the session ID on success.
#[cfg(feature = "telegram")]
#[allow(clippy::too_many_arguments)]
async fn tg_stream(
    bot: &Bot,
    client: &DaemonClient,
    agent: &str,
    chat_id: i64,
    reply_to_msg_id: i64,
    content: &str,
    sender: &str,
    session: Option<u64>,
) -> StreamResult {
    use std::time::Duration;

    let client_msg = ClientMessage::from(StreamMsg {
        agent: agent.to_string(),
        content: content.to_string(),
        session,
        sender: Some(sender.to_string()),
    });
    let mut reply_rx = client.send(client_msg).await;
    let mut acc = StreamAccumulator::new();
    let mut msg_id: Option<teloxide::types::MessageId> = None;
    let mut last_sent_len: usize = 0;
    let mut debounce = tokio::time::interval(Duration::from_millis(1500));
    debounce.reset(); // Don't fire immediately.

    // Start typing indicator right away.
    let typing_bot = bot.clone();
    let typing_handle = tokio::spawn(async move {
        loop {
            if typing_bot
                .send_chat_action(ChatId(chat_id), ChatAction::Typing)
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
    });

    loop {
        tokio::select! {
            server_msg = reply_rx.recv() => {
                match server_msg {
                    Some(ServerMessage { msg: Some(server_message::Msg::Stream(event)) }) => {
                        acc.push(&event);
                        if acc.is_done() {
                            break;
                        }
                    }
                    Some(ServerMessage { msg: Some(server_message::Msg::Error(err)) }) => {
                        acc.set_error(err.message);
                        break;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            _ = debounce.tick() => {
                // Send or edit the message with accumulated text.
                let rendered = acc.render();
                if rendered.is_empty() || rendered.len() == last_sent_len {
                    continue;
                }
                let reply_to = Some(teloxide::types::MessageId(reply_to_msg_id as i32));
                match msg_id {
                    None => {
                        match crate::telegram::markdown::send_md(bot, ChatId(chat_id), &rendered, reply_to).await {
                            Ok(sent) => {
                                msg_id = Some(sent.id);
                                last_sent_len = rendered.len();
                            }
                            Err(e) => tracing::warn!(agent, "failed to send placeholder: {e}"),
                        }
                    }
                    Some(mid) => {
                        if let Err(e) = crate::telegram::markdown::edit_md(bot, ChatId(chat_id), mid, &rendered).await {
                            tracing::debug!(agent, "edit failed (may be same text): {e}");
                        } else {
                            last_sent_len = rendered.len();
                        }
                    }
                }
            }
        }
    }

    // Stop typing indicator.
    typing_handle.abort();

    // Handle errors.
    if let Some(err) = acc.error() {
        tracing::warn!(agent, chat_id, "stream error: {err}");
        let err_text = format!("Error: {err}");
        if let Err(e) = bot.send_message(ChatId(chat_id), err_text).await {
            tracing::warn!(agent, "failed to send error to chat: {e}");
        }
        return if session.is_some() {
            StreamResult::SessionError
        } else {
            StreamResult::Failed
        };
    }

    // Final edit with complete text.
    let final_text = acc.render();
    if !final_text.is_empty() {
        match msg_id {
            Some(mid) if final_text.len() != last_sent_len => {
                if let Err(e) =
                    crate::telegram::markdown::edit_md(bot, ChatId(chat_id), mid, &final_text).await
                {
                    tracing::debug!(agent, "final edit failed: {e}");
                }
            }
            None => {
                let reply_to = Some(teloxide::types::MessageId(reply_to_msg_id as i32));
                if let Err(e) =
                    crate::telegram::markdown::send_md(bot, ChatId(chat_id), &final_text, reply_to)
                        .await
                {
                    tracing::warn!(agent, "failed to send reply: {e}");
                }
            }
            _ => {}
        }
    }

    match acc.session() {
        Some(session_id) => StreamResult::Ok { session_id },
        None => StreamResult::Failed,
    }
}

#[cfg(feature = "discord")]
/// Discord message loop: routes incoming messages to agents or bot commands.
///
/// Maintains a `chat_id → session_id` mapping so consecutive messages from the
/// same chat reuse the same session. Uses `StreamMsg` with `StreamAccumulator`,
/// sends the final accumulated text when the stream completes.
async fn discord_loop(
    mut rx: mpsc::UnboundedReceiver<GatewayMessage>,
    http: Arc<serenity::http::Http>,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let mut sessions: HashMap<i64, u64> = HashMap::new();
    let mut chat_agents: HashMap<i64, CompactString> = HashMap::new();

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

        let active_agent = chat_agents.get(&chat_id).unwrap_or(&agent);
        tracing::info!(agent = %active_agent, chat_id, "discord dispatch");

        // Bot command path.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(crate::command::BotCommand::Switch { agent: new_agent }) => {
                    let new_agent: CompactString = new_agent.into();
                    chat_agents.insert(chat_id, new_agent.clone());
                    sessions.remove(&chat_id);
                    let msg = format!("Switched to agent: {new_agent}");
                    crate::discord::send_text(&http, channel_id, msg).await;
                }
                Some(cmd) => {
                    let h = http.clone();
                    let c = client.clone();
                    tokio::spawn(async move {
                        crate::discord::command::dispatch_command(cmd, c, h, channel_id).await;
                    });
                }
                None => {
                    tracing::warn!(chat_id, content, "unrecognised bot command");
                    crate::discord::send_text(
                        &http,
                        channel_id,
                        crate::command::COMMAND_HINT.to_owned(),
                    )
                    .await;
                }
            }
            continue;
        }

        // Normal agent chat path with session mapping.
        let session = sessions.get(&chat_id).copied();

        // Append attachment summary to content if present.
        let content = match crate::message::attachment_summary(&msg.attachments) {
            Some(summary) => format!("{content}\n{summary}"),
            None => content,
        };

        let result = dc_stream(
            &http,
            &client,
            active_agent,
            channel_id,
            &content,
            &sender,
            session,
        )
        .await;

        match result {
            StreamResult::Ok { session_id } => {
                sessions.insert(chat_id, session_id);
            }
            StreamResult::SessionError if session.is_some() => {
                tracing::warn!(agent = %active_agent, chat_id, "session error, retrying");
                sessions.remove(&chat_id);
                let retry = dc_stream(
                    &http,
                    &client,
                    active_agent,
                    channel_id,
                    &content,
                    &sender,
                    None,
                )
                .await;
                if let StreamResult::Ok { session_id } = retry {
                    sessions.insert(chat_id, session_id);
                }
            }
            StreamResult::SessionError | StreamResult::Failed => {}
        }
    }

    tracing::info!(platform = "discord", "channel loop ended");
}

/// Send a streaming request to the daemon and post the accumulated response
/// to a Discord channel when done.
#[cfg(feature = "discord")]
async fn dc_stream(
    http: &Arc<serenity::http::Http>,
    client: &DaemonClient,
    agent: &str,
    channel_id: ChannelId,
    content: &str,
    sender: &str,
    session: Option<u64>,
) -> StreamResult {
    let client_msg = ClientMessage::from(StreamMsg {
        agent: agent.to_string(),
        content: content.to_string(),
        session,
        sender: Some(sender.to_string()),
    });
    let mut reply_rx = client.send(client_msg).await;
    let mut acc = StreamAccumulator::new();

    while let Some(server_msg) = reply_rx.recv().await {
        match server_msg {
            ServerMessage {
                msg: Some(server_message::Msg::Stream(event)),
            } => {
                acc.push(&event);
                if acc.is_done() {
                    break;
                }
            }
            ServerMessage {
                msg: Some(server_message::Msg::Error(err)),
            } => {
                acc.set_error(err.message);
                break;
            }
            _ => {}
        }
    }

    if let Some(err) = acc.error() {
        tracing::warn!(agent, "discord stream error: {err}");
        crate::discord::send_text(http, channel_id, format!("Error: {err}")).await;
        return if session.is_some() {
            StreamResult::SessionError
        } else {
            StreamResult::Failed
        };
    }

    let final_text = acc.render();
    if !final_text.is_empty() {
        crate::discord::send_text(http, channel_id, final_text).await;
    }

    match acc.session() {
        Some(session_id) => StreamResult::Ok { session_id },
        None => StreamResult::Failed,
    }
}

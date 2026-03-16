//! Telegram gateway serve command.

use compact_str::CompactString;
use gateway::{
    BotCommand, COMMAND_HINT, DaemonClient, GatewayConfig, GatewayMessage, KnownBots,
    StreamAccumulator, StreamResult, attachment_summary, parse_command,
};
use std::{collections::HashMap, path::Path, sync::Arc};
use teloxide::prelude::*;
use teloxide::types::ChatAction;
use tokio::sync::mpsc;
use wcore::protocol::message::{ClientMessage, ServerMessage, StreamMsg, server_message};

/// Run the Telegram gateway service.
pub async fn run(daemon_socket: &str, config_json: &str) -> anyhow::Result<()> {
    let config: GatewayConfig = serde_json::from_str(config_json)?;
    let client = Arc::new(DaemonClient::new(Path::new(daemon_socket)));

    let agents_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    let default_agent = gateway::resolve_default_agent(&agents_dir);
    tracing::info!(agent = %default_agent, "telegram gateway starting");

    let known_bots: KnownBots =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new()));

    if let Some(tg) = &config.telegram {
        if tg.token.is_empty() {
            tracing::warn!(platform = "telegram", "token is empty, skipping");
        } else {
            spawn_telegram(&tg.token, default_agent, client, known_bots).await;
        }
    } else {
        tracing::warn!(platform = "telegram", "no telegram config provided");
    }

    tokio::signal::ctrl_c().await?;
    tracing::info!("telegram gateway shutting down");
    Ok(())
}

async fn spawn_telegram(
    token: &str,
    agent: CompactString,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let bot = Bot::new(token);

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

    // Register slash commands so they appear in the Telegram UI.
    use teloxide::types::BotCommand as TgCommand;
    let commands = vec![
        TgCommand::new("switch", "Switch to a different agent"),
        TgCommand::new("hub", "Manage hub packages (install/uninstall)"),
    ];
    if let Err(e) = bot.set_my_commands(commands).await {
        tracing::warn!(
            platform = "telegram",
            "failed to register bot commands: {e}"
        );
    }

    let (tx, rx) = mpsc::unbounded_channel::<GatewayMessage>();

    let poll_bot = bot.clone();
    tokio::spawn(async move {
        crate::telegram::poll_loop(poll_bot, tx).await;
    });

    tokio::spawn(telegram_loop(rx, bot, agent, client, known_bots));
    tracing::info!(platform = "telegram", "channel transport started");
}

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

        if known_bots.read().await.contains(&sender) {
            tracing::debug!(%sender, chat_id, "dropping message from known bot");
            continue;
        }

        let active_agent = chat_agents.get(&chat_id).unwrap_or(&agent);
        tracing::info!(agent = %active_agent, chat_id, "telegram dispatch");

        if content.starts_with('/') {
            match parse_command(&content) {
                Some(BotCommand::Switch { agent: new_agent }) => {
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
                    if let Err(e) = bot.send_message(ChatId(chat_id), COMMAND_HINT).await {
                        tracing::warn!("failed to send command hint: {e}");
                    }
                }
            }
            continue;
        }

        let session = sessions.get(&chat_id).copied();
        let content = match attachment_summary(&msg.attachments) {
            Some(summary) => format!("{content}\n{summary}"),
            None => content,
        };

        let result = tg_stream(
            &bot,
            &client,
            active_agent,
            chat_id,
            msg.message_id,
            msg.is_group,
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
                let retry = tg_stream(
                    &bot,
                    &client,
                    active_agent,
                    chat_id,
                    msg.message_id,
                    msg.is_group,
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

#[allow(clippy::too_many_arguments)]
async fn tg_stream(
    bot: &Bot,
    client: &DaemonClient,
    agent: &str,
    chat_id: i64,
    reply_to_msg_id: i64,
    is_group: bool,
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
    debounce.reset();

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
                let rendered = acc.render();
                if rendered.is_empty() || rendered.len() == last_sent_len {
                    continue;
                }
                let reply_to = is_group.then_some(teloxide::types::MessageId(reply_to_msg_id as i32));
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

    typing_handle.abort();

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
                let reply_to =
                    is_group.then_some(teloxide::types::MessageId(reply_to_msg_id as i32));
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

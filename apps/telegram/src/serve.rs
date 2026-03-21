//! Telegram gateway serve logic.

use crate::{
    COMMAND_HINT, DaemonClient, GatewayConfig, GatewayMessage, KnownBots, StreamAccumulator,
    StreamResult, attachment_summary, parse_command,
};
use std::{collections::HashMap, path::Path, sync::Arc};
use teloxide::prelude::*;
use teloxide::types::{ChatAction, InlineKeyboardButton, InlineKeyboardMarkup};
use tokio::sync::mpsc;
use wcore::protocol::message::{
    AskQuestion, ClientMessage, ReplyToAsk, ServerMessage, StreamMsg, server_message,
};

/// Run the Telegram gateway service.
pub async fn run(daemon_socket: &str, config: &GatewayConfig) -> anyhow::Result<()> {
    let client = Arc::new(DaemonClient::new(Path::new(daemon_socket)));

    let agents_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    let default_agent = crate::resolve_default_agent(&agents_dir);
    tracing::info!(agent = %default_agent, "telegram gateway starting");

    let known_bots: KnownBots =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new()));

    if let Some(tg) = &config.telegram {
        if tg.token.is_empty() {
            tracing::warn!(platform = "telegram", "token is empty, skipping");
        } else {
            spawn_telegram(
                &tg.token,
                &tg.allowed_users,
                default_agent,
                client,
                known_bots,
            )
            .await;
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
    allowed_users: &[i64],
    agent: String,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
) {
    let bot = Bot::new(token);

    match bot.get_me().await {
        Ok(me) => {
            let bot_sender = format!("tg:{}", me.id.0);
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
        crate::poll_loop(poll_bot, tx).await;
    });

    let allowed: std::collections::HashSet<i64> = allowed_users.iter().copied().collect();
    if !allowed.is_empty() {
        tracing::info!(
            platform = "telegram",
            count = allowed.len(),
            "user whitelist active"
        );
    }
    tokio::spawn(telegram_loop(rx, bot, agent, client, known_bots, allowed));
    tracing::info!(platform = "telegram", "channel transport started");
}

/// Per-chat stream state, tracked while a stream is in flight.
struct ChatStream {
    handle: tokio::task::JoinHandle<StreamResult>,
    session_id: Option<u64>,
    reply_tx: mpsc::UnboundedSender<String>,
}

impl ChatStream {
    fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

/// Reap a finished ChatStream, extracting the session_id on success.
async fn reap_chat(chat: ChatStream) -> Option<u64> {
    match chat.handle.await {
        Ok(StreamResult::Ok { session_id }) => Some(session_id),
        _ => chat.session_id,
    }
}

async fn telegram_loop(
    mut rx: mpsc::UnboundedReceiver<GatewayMessage>,
    bot: Bot,
    agent: String,
    client: Arc<DaemonClient>,
    known_bots: KnownBots,
    allowed_users: std::collections::HashSet<i64>,
) {
    let mut chats: HashMap<i64, ChatStream> = HashMap::new();
    let mut sessions: HashMap<i64, u64> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let content = msg.content.clone();
        let sender = format!("tg:{}", msg.sender_id);

        if known_bots.read().await.contains(&sender) {
            tracing::debug!(%sender, chat_id, "dropping message from known bot");
            continue;
        }

        if !allowed_users.is_empty() && !allowed_users.contains(&msg.sender_id) {
            tracing::debug!(
                sender_id = msg.sender_id,
                chat_id,
                "dropping message from non-allowed user"
            );
            continue;
        }

        // Slash commands are always dispatched immediately.
        if content.starts_with('/') {
            match parse_command(&content) {
                Some(cmd) => {
                    let b = bot.clone();
                    tokio::spawn(async move {
                        crate::command::dispatch_command(cmd, b, chat_id).await;
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

        tracing::info!(agent = %agent, chat_id, "telegram dispatch");

        // Check if there's an active stream for this chat.
        if let Some(chat_stream) = chats.get(&chat_id) {
            if chat_stream.is_finished() {
                let chat_stream = chats.remove(&chat_id).unwrap();
                if let Some(sid) = reap_chat(chat_stream).await {
                    sessions.insert(chat_id, sid);
                }
                // Fall through to spawn a new stream below.
            } else {
                // Stream in flight — forward message. If ask_user is pending,
                // tg_stream will route it as ReplyToAsk. Otherwise it's dropped.
                let _ = chat_stream.reply_tx.send(content);
                continue;
            }
        }

        let session = sessions.get(&chat_id).copied();
        let content = match attachment_summary(&msg.attachments) {
            Some(summary) => format!("{content}\n{summary}"),
            None => content,
        };

        // Spawn the stream as a background task.
        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        let handle = {
            let bot = bot.clone();
            let client = client.clone();
            let agent = agent.clone();
            tokio::spawn(async move {
                let result = tg_stream(
                    &bot,
                    &client,
                    &agent,
                    chat_id,
                    msg.message_id,
                    msg.is_group,
                    &content,
                    &sender,
                    session,
                    reply_rx,
                )
                .await;

                // Handle session retry on error.
                match result {
                    StreamResult::SessionError if session.is_some() => {
                        tracing::warn!(agent = %&agent, chat_id, "session error, retrying");
                        let (_retry_tx, retry_rx) = mpsc::unbounded_channel();
                        tg_stream(
                            &bot,
                            &client,
                            &agent,
                            chat_id,
                            msg.message_id,
                            msg.is_group,
                            &content,
                            &sender,
                            None,
                            retry_rx,
                        )
                        .await
                    }
                    other => other,
                }
            })
        };

        chats.insert(
            chat_id,
            ChatStream {
                handle,
                session_id: session,
                reply_tx,
            },
        );
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
    mut reply_rx: mpsc::UnboundedReceiver<String>,
) -> StreamResult {
    use std::time::Duration;

    let client_msg = ClientMessage::from(StreamMsg {
        agent: agent.to_string(),
        content: content.to_string(),
        session,
        sender: Some(sender.to_string()),
        cwd: None,
    });
    let mut server_rx = client.send(client_msg).await;
    let mut acc = StreamAccumulator::new();
    let mut msg_id: Option<teloxide::types::MessageId> = None;
    let mut last_sent_len: usize = 0;
    let mut debounce = tokio::time::interval(Duration::from_millis(1500));
    debounce.reset();
    let mut pending_ask_questions: Option<Vec<AskQuestion>> = None;
    let mut multi_select_state: HashMap<usize, Vec<usize>> = HashMap::new();

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
            server_msg = server_rx.recv() => {
                match server_msg {
                    Some(ServerMessage { msg: Some(server_message::Msg::Stream(event)) }) => {
                        acc.push(&event);

                        // When ask_user fires, flush text and send inline keyboard.
                        if let Some(questions) = acc.take_pending_questions() {
                            let rendered = acc.render();
                            if !rendered.is_empty() && rendered.len() != last_sent_len {
                                let reply_to = is_group.then_some(teloxide::types::MessageId(reply_to_msg_id as i32));
                                match msg_id {
                                    None => {
                                        if let Ok(sent) = crate::markdown::send_md(bot, ChatId(chat_id), &rendered, reply_to).await {
                                            msg_id = Some(sent.id);
                                            last_sent_len = rendered.len();
                                        }
                                    }
                                    Some(mid) => {
                                        if crate::markdown::edit_md(bot, ChatId(chat_id), mid, &rendered).await.is_ok() {
                                            last_sent_len = rendered.len();
                                        }
                                    }
                                }
                            }
                            // Send each question with an inline keyboard.
                            for (qi, q) in questions.iter().enumerate() {
                                let keyboard = build_ask_keyboard(qi, q);
                                let text = format!("📋 {}\n{}", q.header, q.question);
                                if let Err(e) = bot
                                    .send_message(ChatId(chat_id), text)
                                    .reply_markup(keyboard)
                                    .await
                                {
                                    tracing::warn!(agent, "failed to send ask keyboard: {e}");
                                }
                            }
                            pending_ask_questions = Some(questions);
                        }

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
            reply = reply_rx.recv() => {
                if let Some(reply_content) = reply
                    && let Some(ref questions) = pending_ask_questions
                {
                    // Try to parse as callback data: "ask:qi:oi" or "ask:qi:done"
                    let resolved = if reply_content.starts_with("ask:") {
                        handle_ask_callback(
                            &reply_content,
                            questions,
                            &mut multi_select_state,
                            bot,
                            ChatId(chat_id),
                        ).await
                    } else {
                        // Raw text reply — use as-is for the first question.
                        let mut answers = HashMap::new();
                        if let Some(q) = questions.first() {
                            answers.insert(q.question.clone(), reply_content);
                        }
                        Some(serde_json::to_string(&answers).unwrap_or_default())
                    };

                    if let Some(json_reply) = resolved {
                        if let Some(session_id) = acc.session() {
                            let reply_msg = ClientMessage::from(ReplyToAsk {
                                session: session_id,
                                content: json_reply,
                            });
                            let _ = client.send(reply_msg).await;
                        }
                        pending_ask_questions = None;
                        multi_select_state.clear();
                    }
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
                        match crate::markdown::send_md(bot, ChatId(chat_id), &rendered, reply_to).await {
                            Ok(sent) => {
                                msg_id = Some(sent.id);
                                last_sent_len = rendered.len();
                            }
                            Err(e) => tracing::warn!(agent, "failed to send placeholder: {e}"),
                        }
                    }
                    Some(mid) => {
                        if let Err(e) = crate::markdown::edit_md(bot, ChatId(chat_id), mid, &rendered).await {
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
                    crate::markdown::edit_md(bot, ChatId(chat_id), mid, &final_text).await
                {
                    tracing::debug!(agent, "final edit failed: {e}");
                }
            }
            None => {
                let reply_to =
                    is_group.then_some(teloxide::types::MessageId(reply_to_msg_id as i32));
                if let Err(e) =
                    crate::markdown::send_md(bot, ChatId(chat_id), &final_text, reply_to).await
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

/// Build an inline keyboard for a single question.
fn build_ask_keyboard(question_idx: usize, q: &AskQuestion) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = q
        .options
        .iter()
        .enumerate()
        .map(|(oi, opt)| {
            vec![InlineKeyboardButton::callback(
                opt.label.clone(),
                format!("ask:{question_idx}:{oi}"),
            )]
        })
        .collect();

    if q.multi_select {
        rows.push(vec![InlineKeyboardButton::callback(
            "✓ Done".to_string(),
            format!("ask:{question_idx}:done"),
        )]);
    }

    rows.push(vec![InlineKeyboardButton::callback(
        "Other…".to_string(),
        format!("ask:{question_idx}:other"),
    )]);

    InlineKeyboardMarkup::new(rows)
}

/// Handle an ask callback like "ask:0:1" or "ask:0:done" or "ask:0:other".
///
/// Returns `Some(json_reply)` when the answer is complete (single select picked,
/// or multi-select "Done" pressed). Returns `None` when toggling a multi-select
/// option (waiting for "Done").
async fn handle_ask_callback(
    data: &str,
    questions: &[AskQuestion],
    multi_state: &mut HashMap<usize, Vec<usize>>,
    bot: &Bot,
    chat_id: ChatId,
) -> Option<String> {
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 3 {
        return None;
    }

    let qi: usize = parts[1].parse().ok()?;
    let q = questions.get(qi)?;

    if parts[2] == "other" {
        // Ask the user to type a reply. For now, signal that we need free text.
        if let Err(e) = bot.send_message(chat_id, "Please type your answer:").await {
            tracing::warn!("failed to send other prompt: {e}");
        }
        return None;
    }

    if parts[2] == "done" {
        // Multi-select done — build the answer.
        let selected = multi_state.remove(&qi).unwrap_or_default();
        let labels: Vec<&str> = selected
            .iter()
            .filter_map(|&i| q.options.get(i).map(|o| o.label.as_str()))
            .collect();
        let mut answers = HashMap::new();
        answers.insert(q.question.clone(), labels.join(", "));
        return Some(serde_json::to_string(&answers).unwrap_or_default());
    }

    let oi: usize = parts[2].parse().ok()?;

    if q.multi_select {
        // Toggle the option.
        let entry = multi_state.entry(qi).or_default();
        if let Some(pos) = entry.iter().position(|&i| i == oi) {
            entry.remove(pos);
        } else {
            entry.push(oi);
        }
        // Acknowledge the toggle.
        if let Some(opt) = q.options.get(oi) {
            let selected = entry.contains(&oi);
            let mark = if selected { "☑" } else { "☐" };
            let _ = bot
                .send_message(chat_id, format!("{mark} {}", opt.label))
                .await;
        }
        None
    } else {
        // Single select — answer immediately.
        let label = q
            .options
            .get(oi)
            .map(|o| o.label.clone())
            .unwrap_or_default();
        let mut answers = HashMap::new();
        answers.insert(q.question.clone(), label);
        Some(serde_json::to_string(&answers).unwrap_or_default())
    }
}

//! WeChat gateway serve logic.

use crate::config::WechatConfig;
use crate::{
    ContextTokens, GatewayMessage, NodeClient, StreamAccumulator, StreamResult, UserIdMap,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;
use wcore::protocol::message::{
    ClientMessage, ReplyToAsk, ServerMessage, StreamMsg, server_message,
};

/// Run the WeChat gateway service.
pub async fn run(node_client: NodeClient, config: &WechatConfig) -> anyhow::Result<()> {
    let client = Arc::new(node_client);

    let agents_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    let default_agent = crate::resolve_default_agent(&agents_dir);
    tracing::info!(agent = %default_agent, "wechat gateway starting");

    if config.token.is_empty() {
        tracing::warn!(platform = "wechat", "token is empty, skipping");
    } else {
        spawn_wechat(config, default_agent, client).await;
    }

    tokio::signal::ctrl_c().await?;
    tracing::info!("wechat gateway shutting down");
    Ok(())
}

async fn spawn_wechat(wc: &WechatConfig, agent: String, client: Arc<NodeClient>) {
    let (tx, rx) = mpsc::unbounded_channel::<GatewayMessage>();
    let ctx_tokens: ContextTokens = Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let user_ids: UserIdMap = Arc::new(parking_lot::Mutex::new(HashMap::new()));

    let http = reqwest::Client::new();
    let base_url = wc.base_url.clone();
    let token = wc.token.clone();
    let poll_ctx = ctx_tokens.clone();
    let poll_ids = user_ids.clone();
    tokio::spawn(async move {
        crate::poll_loop(http, base_url, token, tx, poll_ctx, poll_ids).await;
    });

    let allowed: std::collections::HashSet<String> = wc.allowed_users.iter().cloned().collect();
    if !allowed.is_empty() {
        tracing::info!(
            platform = "wechat",
            count = allowed.len(),
            "user whitelist active"
        );
    }

    let base_url = wc.base_url.clone();
    let token = wc.token.clone();
    tokio::spawn(wechat_loop(
        rx, agent, client, ctx_tokens, user_ids, allowed, base_url, token,
    ));
    tracing::info!(platform = "wechat", "channel transport started");
}

/// Per-chat stream state.
struct ChatStream {
    handle: tokio::task::JoinHandle<StreamResult>,
    reply_tx: mpsc::UnboundedSender<String>,
}

impl ChatStream {
    fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

async fn reap_chat(chat: ChatStream) -> bool {
    matches!(chat.handle.await, Ok(StreamResult::Ok))
}

#[allow(clippy::too_many_arguments)]
async fn wechat_loop(
    mut rx: mpsc::UnboundedReceiver<GatewayMessage>,
    agent: String,
    client: Arc<NodeClient>,
    ctx_tokens: ContextTokens,
    user_ids: UserIdMap,
    allowed_users: std::collections::HashSet<String>,
    base_url: String,
    token: String,
) {
    let mut chats: HashMap<i64, ChatStream> = HashMap::new();
    let http = reqwest::Client::new();

    while let Some(msg) = rx.recv().await {
        let chat_id = msg.chat_id;
        let content = msg.content.clone();

        // User whitelist check (using original user ID from reverse map).
        if !allowed_users.is_empty() {
            let user_id = user_ids.lock().get(&chat_id).cloned();
            if let Some(ref uid) = user_id
                && !allowed_users.contains(uid)
            {
                tracing::debug!(user_id = %uid, chat_id, "dropping non-allowed user");
                continue;
            }
        }

        tracing::info!(agent = %agent, chat_id, "wechat dispatch");

        // Check for active stream.
        if let Some(chat_stream) = chats.get(&chat_id) {
            if chat_stream.is_finished() {
                let chat_stream = chats.remove(&chat_id).unwrap();
                reap_chat(chat_stream).await;
            } else {
                let _ = chat_stream.reply_tx.send(content);
                continue;
            }
        }

        let sender = user_ids.lock().get(&chat_id).cloned().unwrap_or_default();

        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        let handle = {
            let client = client.clone();
            let agent = agent.clone();
            let http = http.clone();
            let base_url = base_url.clone();
            let token = token.clone();
            let ctx_tokens = ctx_tokens.clone();
            let user_ids = user_ids.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                wx_stream(
                    &http,
                    &client,
                    &agent,
                    chat_id,
                    &content,
                    &sender,
                    reply_rx,
                    &base_url,
                    &token,
                    &ctx_tokens,
                    &user_ids,
                )
                .await
            })
        };

        chats.insert(chat_id, ChatStream { handle, reply_tx });
    }

    tracing::info!(platform = "wechat", "channel loop ended");
}

#[allow(clippy::too_many_arguments)]
async fn wx_stream(
    http: &reqwest::Client,
    client: &NodeClient,
    agent: &str,
    chat_id: i64,
    content: &str,
    sender: &str,
    mut reply_rx: mpsc::UnboundedReceiver<String>,
    base_url: &str,
    token: &str,
    ctx_tokens: &ContextTokens,
    user_ids: &UserIdMap,
) -> StreamResult {
    tracing::info!(agent, chat_id, %sender, "starting stream");
    let client_msg = ClientMessage::from(StreamMsg {
        agent: agent.to_string(),
        content: content.to_string(),
        sender: Some(sender.to_string()),
        cwd: None,
        guest: None,
        tool_choice: None,
    });
    let mut server_rx = client.send(client_msg).await;
    let mut acc = StreamAccumulator::new();

    loop {
        tokio::select! {
            server_msg = server_rx.recv() => {
                match server_msg {
                    Some(ServerMessage { msg: Some(server_message::Msg::Stream(event)) }) => {
                        acc.push(&event);

                        // Handle ask_user: send question text, accept free-text reply.
                        if let Some(questions) = acc.take_pending_questions() {
                            let question_text = questions
                                .iter()
                                .map(|q| format!("{}: {}", q.header, q.question))
                                .collect::<Vec<_>>()
                                .join("\n");

                            let to_user = user_ids.lock().get(&chat_id).cloned();
                            let ctx = ctx_tokens.lock().get(
                                to_user.as_deref().unwrap_or("")
                            ).cloned();

                            if let (Some(to), Some(ct)) = (to_user, ctx) {
                                let _ = crate::api::send_message(
                                    http, base_url, token, &to, &ct, &question_text,
                                ).await;
                            }
                        }

                        if acc.done {
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
                if let Some(reply_content) = reply {
                    // Free-text reply for ask_user.
                    let reply_msg = ClientMessage::from(ReplyToAsk {
                        agent: agent.to_string(),
                        sender: sender.to_string(),
                        content: reply_content,
                    });
                    let _ = client.send(reply_msg).await;
                }
            }
        }
    }

    // Send final response.
    if let Some(err) = acc.error() {
        tracing::warn!(agent, chat_id, "stream error: {err}");
        let to_user = user_ids.lock().get(&chat_id).cloned();
        let ctx = ctx_tokens
            .lock()
            .get(to_user.as_deref().unwrap_or(""))
            .cloned();
        if let (Some(to), Some(ct)) = (to_user, ctx) {
            let _ =
                crate::api::send_message(http, base_url, token, &to, &ct, &format!("Error: {err}"))
                    .await;
        }
        return StreamResult::Failed;
    }

    let final_text = acc.render();
    if !final_text.is_empty() {
        tracing::info!(agent, chat_id, len = final_text.len(), "sending reply");
        let to_user = user_ids.lock().get(&chat_id).cloned();
        let ctx = ctx_tokens
            .lock()
            .get(to_user.as_deref().unwrap_or(""))
            .cloned();
        if let (Some(to), Some(ct)) = (to_user, ctx) {
            if let Err(e) =
                crate::api::send_message(http, base_url, token, &to, &ct, &final_text).await
            {
                tracing::warn!(agent, chat_id, "failed to send reply: {e}");
            } else {
                tracing::info!(agent, chat_id, "reply sent");
            }
        } else {
            tracing::warn!(agent, chat_id, "no user_id or context_token for reply");
        }
    } else {
        tracing::debug!(agent, chat_id, "stream ended with empty response");
    }

    if acc.agent.is_some() {
        tracing::info!(agent, chat_id, "stream completed");
        StreamResult::Ok
    } else {
        tracing::warn!(agent, chat_id, "stream completed without agent");
        StreamResult::Failed
    }
}

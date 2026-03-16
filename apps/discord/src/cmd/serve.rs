//! Discord gateway serve command.

use compact_str::CompactString;
use gateway::{
    BotCommand, COMMAND_HINT, DaemonClient, GatewayConfig, GatewayMessage, KnownBots,
    StreamAccumulator, StreamResult, attachment_summary, parse_command,
};
use serenity::model::id::ChannelId;
use std::{collections::HashMap, path::Path, sync::Arc};
use tokio::sync::mpsc;
use wcore::protocol::message::{ClientMessage, ServerMessage, StreamMsg, server_message};

/// Run the Discord gateway service.
pub async fn run(daemon_socket: &str, config_json: &str) -> anyhow::Result<()> {
    let config: GatewayConfig = serde_json::from_str(config_json)?;
    let client = Arc::new(DaemonClient::new(Path::new(daemon_socket)));

    let agents_dir = wcore::paths::CONFIG_DIR.join(wcore::paths::AGENTS_DIR);
    let default_agent = gateway::resolve_default_agent(&agents_dir);
    tracing::info!(agent = %default_agent, "discord gateway starting");

    let known_bots: KnownBots =
        Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new()));

    if let Some(dc) = &config.discord {
        if dc.token.is_empty() {
            tracing::warn!(platform = "discord", "token is empty, skipping");
        } else {
            spawn_discord(&dc.token, default_agent, client, known_bots).await;
        }
    } else {
        tracing::warn!(platform = "discord", "no discord config provided");
    }

    tokio::signal::ctrl_c().await?;
    tracing::info!("discord gateway shutting down");
    Ok(())
}

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

        if known_bots.read().await.contains(&sender) {
            tracing::debug!(%sender, chat_id, "dropping message from known bot");
            continue;
        }

        let active_agent = chat_agents.get(&chat_id).unwrap_or(&agent);
        tracing::info!(agent = %active_agent, chat_id, "discord dispatch");

        if content.starts_with('/') {
            match parse_command(&content) {
                Some(BotCommand::Switch { agent: new_agent }) => {
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
                    crate::discord::send_text(&http, channel_id, COMMAND_HINT.to_owned()).await;
                }
            }
            continue;
        }

        let session = sessions.get(&chat_id).copied();
        let content = match attachment_summary(&msg.attachments) {
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

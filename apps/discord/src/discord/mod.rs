//! Discord Bot adapter.
//!
//! Uses serenity gateway (WebSocket) for receiving messages and
//! `ChannelId::say` for sending replies.

pub mod command;

use compact_str::CompactString;
use gateway::message::{Attachment, AttachmentKind, GatewayMessage};
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::{RwLock, mpsc, oneshot};

/// Serenity event handler that forwards messages as [`GatewayMessage`]s.
struct Handler {
    tx: mpsc::UnboundedSender<GatewayMessage>,
    http_tx: std::sync::Mutex<Option<oneshot::Sender<Arc<serenity::http::Http>>>>,
    known_bots: Arc<RwLock<HashSet<CompactString>>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, msg: Message) {
        if let Some(cm) = convert_message(msg)
            && self.tx.send(cm).is_err()
        {
            tracing::info!("channel handle dropped, stopping discord event loop");
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        let bot_sender: CompactString = format!("dc:{}", ready.user.id.get()).into();
        tracing::info!(user = %ready.user.name, %bot_sender, "discord bot connected");
        self.known_bots.write().await.insert(bot_sender);
        if let Some(tx) = self.http_tx.lock().unwrap().take() {
            let _ = tx.send(ctx.http.clone());
        }
    }
}

/// Convert a serenity `Message` to a `GatewayMessage`.
fn convert_message(msg: Message) -> Option<GatewayMessage> {
    let chat_id = msg.channel_id.get() as i64;
    let sender_id = msg.author.id.get() as i64;
    let sender_name = CompactString::from(msg.author.name.as_str());
    let is_bot = msg.author.bot;
    let is_group = msg.guild_id.is_some();
    let content = msg.content.clone();

    let attachments = msg
        .attachments
        .iter()
        .map(|a| {
            let kind = match a.content_type.as_deref() {
                Some(ct) if ct.starts_with("image/") => AttachmentKind::Image,
                Some(ct) if ct.starts_with("audio/") => AttachmentKind::Audio,
                Some(ct) if ct.starts_with("video/") => AttachmentKind::Video,
                _ => AttachmentKind::File,
            };
            Attachment {
                kind,
                url: a.url.clone(),
                name: Some(a.filename.clone()),
            }
        })
        .collect();

    Some(GatewayMessage {
        chat_id,
        message_id: msg.id.get() as i64,
        sender_id,
        sender_name,
        is_bot,
        is_group,
        content,
        attachments,
        reply_to: None,
        timestamp: msg.timestamp.unix_timestamp() as u64,
    })
}

/// Start the serenity gateway client.
pub async fn event_loop(
    token: &str,
    tx: mpsc::UnboundedSender<GatewayMessage>,
    http_tx: oneshot::Sender<Arc<serenity::http::Http>>,
    known_bots: Arc<RwLock<HashSet<CompactString>>>,
) {
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let handler = Handler {
        tx,
        http_tx: std::sync::Mutex::new(Some(http_tx)),
        known_bots,
    };

    let mut client = match Client::builder(token, intents).event_handler(handler).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to create discord client: {e}");
            return;
        }
    };

    if let Err(e) = client.start().await {
        tracing::error!("discord gateway error: {e}");
    }
}

/// Send a plain-text message to the channel.
pub async fn send_text(http: &Arc<serenity::http::Http>, channel_id: ChannelId, content: String) {
    if let Err(e) = channel_id.say(http, content).await {
        tracing::warn!("failed to send discord reply: {e}");
    }
}

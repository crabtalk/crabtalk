//! Crabtalk Telegram gateway — Telegram Bot API adapter.

pub mod command;
pub mod config;
pub mod markdown;
pub mod serve;

use futures_util::StreamExt;
pub use sdk::*;
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatKind, UpdateKind};
use teloxide::update_listeners::{AsUpdateStream, polling_default};
use tokio::sync::mpsc;

/// Long-poll loop: receives Telegram updates and forwards them as [`GatewayMessage`]s.
pub async fn poll_loop(bot: Bot, tx: mpsc::UnboundedSender<GatewayMessage>) {
    let mut listener = polling_default(bot).await;
    let stream = listener.as_stream();
    futures_util::pin_mut!(stream);

    while let Some(result) = stream.next().await {
        match result {
            Ok(update) => {
                if let Some(msg) = convert_update(update)
                    && tx.send(msg).is_err()
                {
                    tracing::info!("channel handle dropped, stopping poll loop");
                    return;
                }
            }
            Err(e) => {
                tracing::error!("telegram update error: {e}");
            }
        }
    }
}

/// Convert a teloxide `Update` to a `GatewayMessage`.
fn convert_update(update: Update) -> Option<GatewayMessage> {
    match update.kind {
        UpdateKind::CallbackQuery(cq) => convert_callback_query(cq),
        UpdateKind::Message(msg) => convert_message(msg),
        _ => None,
    }
}

/// Convert a `CallbackQuery` into a `GatewayMessage` carrying callback data as content.
fn convert_callback_query(cq: CallbackQuery) -> Option<GatewayMessage> {
    let data = cq.data?;
    let msg_ref = cq.message?;
    let chat = msg_ref.chat().clone();
    let msg_id = msg_ref.id();
    let sender_id = cq.from.id.0 as i64;
    let sender_name = cq.from.first_name.clone();
    let is_bot = cq.from.is_bot;
    let is_group = matches!(chat.kind, ChatKind::Public(_));

    Some(GatewayMessage {
        chat_id: chat.id.0,
        message_id: msg_id.0 as i64,
        sender_id,
        sender_name,
        is_bot,
        is_group,
        content: data,
        attachments: Vec::new(),
        reply_to: None,
        timestamp: 0,
    })
}

/// Convert a regular `Message` update into a `GatewayMessage`.
fn convert_message(msg: teloxide::types::Message) -> Option<GatewayMessage> {
    let chat_id = msg.chat.id.0;
    let sender = msg.from.as_ref();
    let sender_id = sender.map(|u| u.id.0 as i64).unwrap_or(0);
    let sender_name = sender.map(|u| u.first_name.clone()).unwrap_or_default();
    let is_bot = sender.is_some_and(|u| u.is_bot);
    let is_group = matches!(msg.chat.kind, ChatKind::Public(_));
    let content = msg.text().unwrap_or("").to_owned();

    let mut attachments = Vec::new();
    if let Some(photos) = msg.photo()
        && let Some(largest) = photos.last()
    {
        attachments.push(Attachment {
            kind: AttachmentKind::Image,
            url: largest.file.id.0.clone(),
            name: None,
        });
    }
    if let Some(doc) = msg.document() {
        attachments.push(Attachment {
            kind: AttachmentKind::File,
            url: doc.file.id.0.clone(),
            name: doc.file_name.clone(),
        });
    }

    let reply_to = msg.reply_to_message().map(|r| r.id.0);

    Some(GatewayMessage {
        chat_id,
        message_id: msg.id.0 as i64,
        sender_id,
        sender_name,
        is_bot,
        is_group,
        content,
        attachments,
        reply_to,
        timestamp: msg.date.timestamp() as u64,
    })
}

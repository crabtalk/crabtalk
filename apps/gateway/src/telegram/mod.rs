//! Telegram Bot API adapter.
//!
//! Uses teloxide long-polling for receiving messages and `Bot::send_message`
//! for sending replies.

pub(crate) mod command;

use crate::message::{Attachment, AttachmentKind, GatewayMessage};
use compact_str::CompactString;
use futures_util::StreamExt;
use teloxide::prelude::*;
use teloxide::types::{ChatKind, UpdateKind};
use teloxide::update_listeners::{AsUpdateStream, polling_default};
use tokio::sync::mpsc;

/// Long-poll loop: receives Telegram updates and forwards them as [`GatewayMessage`]s.
pub(crate) async fn poll_loop(bot: Bot, tx: mpsc::UnboundedSender<GatewayMessage>) {
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
    let UpdateKind::Message(msg) = update.kind else {
        return None;
    };

    let chat_id = msg.chat.id.0;
    let sender = msg.from.as_ref();
    let sender_id = sender.map(|u| u.id.0 as i64).unwrap_or(0);
    let sender_name = sender
        .map(|u| CompactString::from(u.first_name.as_str()))
        .unwrap_or_default();
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

//! Telegram MarkdownV2 helpers.
//!
//! Provides send/edit wrappers that try `MarkdownV2` parse mode first and
//! fall back to plain text on parse errors.

use teloxide::{
    prelude::*,
    types::{MessageId, ParseMode, ReplyParameters},
};

/// Characters that must be escaped in MarkdownV2 text (outside code spans).
const SPECIAL_CHARS: &[char] = &[
    '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
];

/// Escape special characters for Telegram MarkdownV2.
pub fn escape_markdown_v2(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for ch in text.chars() {
        if SPECIAL_CHARS.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Send a new message with MarkdownV2, falling back to plain text on error.
pub async fn send_md(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    reply_to: Option<MessageId>,
) -> Result<teloxide::types::Message, teloxide::RequestError> {
    let escaped = escape_markdown_v2(text);
    let mut req = bot
        .send_message(chat_id, &escaped)
        .parse_mode(ParseMode::MarkdownV2);
    if let Some(mid) = reply_to {
        req = req.reply_parameters(ReplyParameters::new(mid));
    }
    match req.await {
        Ok(msg) => Ok(msg),
        Err(e) => {
            tracing::debug!("MarkdownV2 send failed, falling back to plain: {e}");
            let mut req = bot.send_message(chat_id, text);
            if let Some(mid) = reply_to {
                req = req.reply_parameters(ReplyParameters::new(mid));
            }
            req.await
        }
    }
}

/// Edit an existing message with MarkdownV2, falling back to plain text on error.
pub async fn edit_md(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    text: &str,
) -> Result<teloxide::types::Message, teloxide::RequestError> {
    let escaped = escape_markdown_v2(text);
    match bot
        .edit_message_text(chat_id, message_id, &escaped)
        .parse_mode(ParseMode::MarkdownV2)
        .await
    {
        Ok(msg) => Ok(msg),
        Err(e) => {
            tracing::debug!("MarkdownV2 edit failed, falling back to plain: {e}");
            bot.edit_message_text(chat_id, message_id, text).await
        }
    }
}

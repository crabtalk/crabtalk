//! Walrus Telegram channel adapter.
//!
//! Connects agents to Telegram via the Bot API using reqwest directly (DD#2).
//! Implements the [`Channel`] trait from walrus-core.

use agent::{Attachment, AttachmentKind, Channel, ChannelMessage, Platform};
use anyhow::Result;
use compact_str::CompactString;
use futures_core::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, Ordering};

/// Telegram Bot API channel adapter.
///
/// Uses long-polling `getUpdates` for receiving messages and
/// `sendMessage` for sending.
pub struct TelegramChannel {
    /// Bot API token.
    bot_token: CompactString,
    /// HTTP client for API calls.
    client: Client,
    /// Long-poll timeout in seconds.
    poll_timeout: u64,
    /// Last processed update_id for deduplication.
    last_update_id: AtomicI64,
}

impl TelegramChannel {
    /// Create a new TelegramChannel with the given bot token.
    pub fn new(bot_token: impl Into<CompactString>) -> Self {
        Self {
            bot_token: bot_token.into(),
            client: Client::new(),
            poll_timeout: 30,
            last_update_id: AtomicI64::new(0),
        }
    }

    /// Create with a custom reqwest client and poll timeout.
    pub fn with_config(
        bot_token: impl Into<CompactString>,
        client: Client,
        poll_timeout: u64,
    ) -> Self {
        Self {
            bot_token: bot_token.into(),
            client,
            poll_timeout,
            last_update_id: AtomicI64::new(0),
        }
    }

    /// Base URL for Telegram Bot API requests.
    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.bot_token)
    }
}

/// Telegram getUpdates response.
#[derive(Debug, Deserialize)]
struct GetUpdatesResponse {
    ok: bool,
    #[serde(default)]
    result: Vec<Update>,
}

/// A single Telegram update.
#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    #[serde(default)]
    message: Option<TelegramMessage>,
}

/// A Telegram message within an update.
#[derive(Debug, Deserialize)]
struct TelegramMessage {
    #[serde(default)]
    message_id: i64,
    #[serde(default)]
    date: u64,
    #[serde(default)]
    chat: TelegramChat,
    #[serde(default)]
    from: Option<TelegramUser>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    reply_to_message: Option<Box<TelegramMessage>>,
    #[serde(default)]
    photo: Option<Vec<PhotoSize>>,
    #[serde(default)]
    document: Option<Document>,
}

/// Telegram chat info.
#[derive(Debug, Default, Deserialize)]
struct TelegramChat {
    id: i64,
}

/// Telegram user info.
#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
}

/// Telegram photo size (smallest to largest).
#[derive(Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
}

/// Telegram document attachment.
#[derive(Debug, Deserialize)]
struct Document {
    file_id: String,
    #[serde(default)]
    file_name: Option<String>,
}

/// Convert a Telegram Update to a ChannelMessage.
pub fn channel_message_from_update(update: &serde_json::Value) -> Option<ChannelMessage> {
    let update: Update = serde_json::from_value(update.clone()).ok()?;
    let msg = update.message?;
    convert_message(&msg)
}

/// Convert internal TelegramMessage to ChannelMessage.
fn convert_message(msg: &TelegramMessage) -> Option<ChannelMessage> {
    let content = msg.text.clone().unwrap_or_default();
    let sender_id = msg
        .from
        .as_ref()
        .map(|u| CompactString::from(u.id.to_string()))
        .unwrap_or_default();

    let mut attachments = Vec::new();
    if let Some(photos) = &msg.photo
        && let Some(largest) = photos.last()
    {
        attachments.push(Attachment {
            kind: AttachmentKind::Image,
            url: largest.file_id.clone(),
            name: None,
        });
    }
    if let Some(doc) = &msg.document {
        attachments.push(Attachment {
            kind: AttachmentKind::File,
            url: doc.file_id.clone(),
            name: doc.file_name.clone(),
        });
    }

    let reply_to = msg
        .reply_to_message
        .as_ref()
        .map(|r| CompactString::from(r.message_id.to_string()));

    Some(ChannelMessage {
        platform: Platform::Telegram,
        channel_id: CompactString::from(msg.chat.id.to_string()),
        sender_id,
        content,
        attachments,
        reply_to,
        timestamp: msg.date,
    })
}

impl Channel for TelegramChannel {
    type Event = ChannelMessage;
    type Config = ();

    fn platform(&self) -> Platform {
        Platform::Telegram
    }

    fn connect(
        &self,
        _config: Self::Config,
    ) -> impl std::future::Future<Output = Result<impl Stream<Item = Self::Event> + Send>> + Send
    {
        let client = self.client.clone();
        let url = self.api_url("getUpdates");
        let timeout = self.poll_timeout;
        let last_update_id = &self.last_update_id;

        async move {
            let stream = async_stream::stream! {
                loop {
                    let offset = last_update_id.load(Ordering::Relaxed) + 1;
                    let params = serde_json::json!({
                        "offset": offset,
                        "timeout": timeout,
                    });

                    let resp = match client.post(&url).json(&params).send().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!("getUpdates failed: {e}");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    };

                    let body: GetUpdatesResponse = match resp.json::<GetUpdatesResponse>().await {
                        Ok(b) if b.ok => b,
                        Ok(_) => {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                        Err(e) => {
                            tracing::error!("getUpdates parse failed: {e}");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    };

                    for update in &body.result {
                        last_update_id.store(update.update_id, Ordering::Relaxed);
                        if let Some(msg) = &update.message
                            && let Some(channel_msg) = convert_message(msg)
                        {
                            yield channel_msg;
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            };

            Ok(stream)
        }
    }

    fn send(
        &self,
        message: ChannelMessage,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let client = self.client.clone();
        let url = self.api_url("sendMessage");
        let chat_id = message.channel_id.to_string();
        let text = message.content;

        async move {
            let params = serde_json::json!({
                "chat_id": chat_id,
                "text": text,
            });

            let resp = client.post(&url).json(&params).send().await?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("sendMessage failed: {body}");
            }

            Ok(())
        }
    }
}

/// Construct the API URL for a given method and bot token.
pub fn send_message_url(bot_token: &str) -> String {
    format!("https://api.telegram.org/bot{bot_token}/sendMessage")
}

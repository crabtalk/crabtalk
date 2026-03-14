//! Gateway message types.

use compact_str::CompactString;

/// A message received from or sent to a gateway.
#[derive(Debug, Clone)]
pub struct GatewayMessage {
    /// Platform chat/channel ID.
    pub chat_id: i64,
    /// Platform sender user ID.
    pub sender_id: i64,
    /// Display name of the sender.
    pub sender_name: CompactString,
    /// Whether the sender is a bot.
    pub is_bot: bool,
    /// Whether this message is from a group chat (vs DM).
    pub is_group: bool,
    /// Message text content.
    pub content: String,
    /// Attached files or media.
    pub attachments: Vec<Attachment>,
    /// Message ID being replied to, if any.
    pub reply_to: Option<i32>,
    /// Unix timestamp when the message was created.
    pub timestamp: u64,
}

/// A file or media attachment.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Type of attachment.
    pub kind: AttachmentKind,
    /// URL or path to the attachment.
    pub url: String,
    /// Optional human-readable name.
    pub name: Option<String>,
}

/// Type of attachment content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    /// Image file (PNG, JPG, etc.).
    Image,
    /// Generic file.
    File,
    /// Audio file.
    Audio,
    /// Video file.
    Video,
}

impl From<GatewayMessage> for wcore::model::Message {
    fn from(msg: GatewayMessage) -> Self {
        wcore::model::Message::user(msg.content)
    }
}

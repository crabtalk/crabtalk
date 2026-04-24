//! Gateway message types.

/// A message received from or sent to a gateway.
#[derive(Debug, Clone)]
pub struct GatewayMessage {
    /// Platform chat/channel ID.
    pub chat_id: i64,
    /// Platform-specific message ID (for reply threading).
    pub message_id: i64,
    /// Platform sender user ID.
    pub sender_id: i64,
    /// Display name of the sender.
    pub sender_name: String,
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

/// Build an attachment summary line like `[Attachments: 2 images, 1 file]`.
///
/// Returns `None` if the list is empty.
pub fn attachment_summary(attachments: &[Attachment]) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    let mut images = 0u32;
    let mut files = 0u32;
    let mut audio = 0u32;
    let mut video = 0u32;
    for a in attachments {
        match a.kind {
            AttachmentKind::Image => images += 1,
            AttachmentKind::File => files += 1,
            AttachmentKind::Audio => audio += 1,
            AttachmentKind::Video => video += 1,
        }
    }
    let mut parts = Vec::new();
    if images > 0 {
        parts.push(format!(
            "{images} image{}",
            if images > 1 { "s" } else { "" }
        ));
    }
    if files > 0 {
        parts.push(format!("{files} file{}", if files > 1 { "s" } else { "" }));
    }
    if audio > 0 {
        parts.push(format!("{audio} audio"));
    }
    if video > 0 {
        parts.push(format!("{video} video"));
    }
    Some(format!("[Attachments: {}]", parts.join(", ")))
}

//! Tests for the Telegram channel adapter.

use agent::{Channel, Platform};
use walrus_telegram::{TelegramChannel, channel_message_from_update, send_message_url};

#[test]
fn telegram_channel_platform() {
    let channel = TelegramChannel::new("test-token");
    assert_eq!(channel.platform(), Platform::Telegram);
}

#[test]
fn telegram_channel_construction() {
    let channel = TelegramChannel::with_config("bot123:ABC", reqwest::Client::new(), 60);
    assert_eq!(channel.platform(), Platform::Telegram);
}

#[test]
fn channel_message_from_update_parses() {
    let update = serde_json::json!({
        "update_id": 100,
        "message": {
            "message_id": 1,
            "date": 1700000000_u64,
            "chat": { "id": 42 },
            "from": { "id": 99, "is_bot": false, "first_name": "Test" },
            "text": "Hello bot"
        }
    });

    let msg = channel_message_from_update(&update).unwrap();
    assert_eq!(msg.platform, Platform::Telegram);
    assert_eq!(msg.channel_id, "42");
    assert_eq!(msg.sender_id, "99");
    assert_eq!(msg.content, "Hello bot");
    assert!(msg.attachments.is_empty());
    assert_eq!(msg.timestamp, 1700000000);
}

#[test]
fn channel_message_from_update_with_photo() {
    let update = serde_json::json!({
        "update_id": 101,
        "message": {
            "message_id": 2,
            "date": 1700000001_u64,
            "chat": { "id": 42 },
            "from": { "id": 99, "is_bot": false, "first_name": "Test" },
            "text": "",
            "photo": [
                { "file_id": "small_id", "file_unique_id": "x", "width": 90, "height": 90, "file_size": 1000 },
                { "file_id": "large_id", "file_unique_id": "y", "width": 800, "height": 600, "file_size": 50000 }
            ]
        }
    });

    let msg = channel_message_from_update(&update).unwrap();
    assert_eq!(msg.attachments.len(), 1);
    assert_eq!(msg.attachments[0].url, "large_id");
}

#[test]
fn channel_message_from_update_missing_message() {
    let update = serde_json::json!({ "update_id": 102 });
    assert!(channel_message_from_update(&update).is_none());
}

#[test]
fn send_message_url_format() {
    let url = send_message_url("bot123:ABC");
    assert_eq!(url, "https://api.telegram.org/botbot123:ABC/sendMessage");
}

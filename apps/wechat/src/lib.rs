//! Crabtalk WeChat gateway — ilink bot API adapter.

pub mod api;
pub mod serve;

pub use gateway::*;

use api::WeixinMessage;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;

/// Error code returned by the server when the bot session has expired.
const SESSION_EXPIRED_ERRCODE: i32 = -14;

/// Shared context token cache: from_user_id → last context_token.
pub type ContextTokens = Arc<Mutex<HashMap<String, String>>>;

/// Shared reverse map: chat_id (hash) → from_user_id string.
pub type UserIdMap = Arc<Mutex<HashMap<i64, String>>>;

/// Sync buffer persistence path.
fn sync_buf_path() -> PathBuf {
    wcore::paths::RUN_DIR.join("wechat_sync.json")
}

fn load_sync_buf() -> String {
    std::fs::read_to_string(sync_buf_path()).unwrap_or_default()
}

fn save_sync_buf(buf: &str) {
    if let Some(parent) = sync_buf_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(sync_buf_path(), buf);
}

/// Stable hash of a string user ID to i64 for GatewayMessage compatibility.
fn hash_user_id(user_id: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    user_id.hash(&mut hasher);
    hasher.finish() as i64
}

/// Extract text content from a WeixinMessage.
fn extract_text(msg: &WeixinMessage) -> String {
    msg.item_list
        .iter()
        .filter(|item| item.type_ == 1) // TEXT
        .filter_map(|item| item.text_item.as_ref()?.text.as_deref())
        .collect::<Vec<_>>()
        .join("")
}

/// Long-poll loop: receives WeChat messages and forwards them as [`GatewayMessage`]s.
pub async fn poll_loop(
    client: reqwest::Client,
    base_url: String,
    token: String,
    tx: mpsc::UnboundedSender<GatewayMessage>,
    ctx_tokens: ContextTokens,
    user_ids: UserIdMap,
) {
    let mut buf = load_sync_buf();
    tracing::info!(
        base_url = %base_url,
        sync_buf_len = buf.len(),
        "poll loop starting"
    );

    loop {
        tracing::debug!("polling getupdates");
        match api::get_updates(&client, &base_url, &token, &buf).await {
            Ok(resp) => {
                // Check for API-level errors.
                let errcode = resp.errcode.unwrap_or(0);
                let ret = resp.ret;
                if errcode != 0 || ret != 0 {
                    let code = if errcode != 0 { errcode } else { ret };
                    tracing::warn!(
                        code,
                        errmsg = resp.errmsg.as_deref().unwrap_or(""),
                        "getupdates error"
                    );

                    if code == SESSION_EXPIRED_ERRCODE {
                        // Session expired — reset sync buf and pause before retry.
                        tracing::error!(
                            "bot session expired (errcode {code}), resetting sync buf, pausing 30s"
                        );
                        buf.clear();
                        save_sync_buf(&buf);
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    continue;
                }

                if !resp.msgs.is_empty() {
                    tracing::info!(count = resp.msgs.len(), "received messages");
                }

                for msg in &resp.msgs {
                    // Skip bot messages (message_type 2 = BOT).
                    if msg.message_type == 2 {
                        tracing::debug!(from = %msg.from_user_id, "skipping bot message");
                        continue;
                    }

                    let text = extract_text(msg);
                    if text.is_empty() {
                        tracing::debug!(from = %msg.from_user_id, "skipping empty message");
                        continue;
                    }

                    tracing::info!(
                        from = %msg.from_user_id,
                        len = text.len(),
                        has_context_token = msg.context_token.is_some(),
                        "inbound message"
                    );

                    let chat_id = hash_user_id(&msg.from_user_id);

                    // Cache context token and user ID for replies.
                    if let Some(ref ct) = msg.context_token {
                        ctx_tokens
                            .lock()
                            .unwrap()
                            .insert(msg.from_user_id.clone(), ct.clone());
                    }
                    user_ids
                        .lock()
                        .unwrap()
                        .insert(chat_id, msg.from_user_id.clone());

                    let gateway_msg = GatewayMessage {
                        chat_id,
                        message_id: 0,
                        sender_id: chat_id,
                        sender_name: msg.from_user_id.clone(),
                        is_bot: false,
                        is_group: false,
                        content: text,
                        attachments: vec![],
                        reply_to: None,
                        timestamp: msg.create_time_ms.unwrap_or(0) / 1000,
                    };

                    if tx.send(gateway_msg).is_err() {
                        tracing::info!("channel dropped, stopping wechat poll loop");
                        return;
                    }
                }

                if let Some(new_buf) = resp.get_updates_buf
                    && new_buf != buf
                {
                    tracing::debug!(len = new_buf.len(), "sync buf updated");
                    buf = new_buf;
                    save_sync_buf(&buf);
                }
            }
            Err(e) => {
                tracing::error!("getupdates failed: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

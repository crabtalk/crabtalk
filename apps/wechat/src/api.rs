//! WeChat ilink bot HTTP API client.

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rand::Rng;
use reqwest::{Client, header::HeaderMap};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);
const API_TIMEOUT: Duration = Duration::from_secs(15);

/// Build common headers for ilink API requests.
fn build_headers(token: &str) -> HeaderMap {
    let uin: u32 = rand::rng().random();
    let uin_b64 = BASE64.encode(uin.to_string());

    let mut headers = HeaderMap::new();
    headers.insert("AuthorizationType", "ilink_bot_token".parse().unwrap());
    headers.insert("Authorization", format!("Bearer {token}").parse().unwrap());
    headers.insert("X-WECHAT-UIN", uin_b64.parse().unwrap());
    headers
}

// ── Request / response types ────────────────────────────────────────

#[derive(Serialize)]
struct BaseInfo {
    channel_version: &'static str,
}

fn base_info() -> BaseInfo {
    BaseInfo {
        channel_version: env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Serialize)]
struct GetUpdatesReq {
    get_updates_buf: String,
    base_info: BaseInfo,
}

#[derive(Debug, Deserialize)]
pub struct GetUpdatesResp {
    #[serde(default)]
    pub ret: i32,
    #[serde(default)]
    pub errcode: Option<i32>,
    #[serde(default)]
    pub errmsg: Option<String>,
    #[serde(default)]
    pub msgs: Vec<WeixinMessage>,
    pub get_updates_buf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WeixinMessage {
    #[serde(default)]
    pub from_user_id: String,
    #[serde(default)]
    pub to_user_id: String,
    pub context_token: Option<String>,
    #[serde(default)]
    pub message_type: i32,
    #[serde(default)]
    pub message_state: i32,
    #[serde(default)]
    pub item_list: Vec<MessageItem>,
    #[serde(default)]
    pub create_time_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct MessageItem {
    #[serde(rename = "type", default)]
    pub type_: i32,
    pub text_item: Option<TextItem>,
}

#[derive(Debug, Deserialize)]
pub struct TextItem {
    pub text: Option<String>,
}

#[derive(Serialize)]
struct SendMessageReqBody {
    msg: SendMessageMsg,
    base_info: BaseInfo,
}

#[derive(Serialize)]
struct SendMessageMsg {
    to_user_id: String,
    context_token: String,
    message_type: i32,
    message_state: i32,
    item_list: Vec<SendMessageItem>,
}

#[derive(Serialize)]
struct SendMessageItem {
    #[serde(rename = "type")]
    type_: i32,
    text_item: SendTextItem,
}

#[derive(Serialize)]
struct SendTextItem {
    text: String,
}

// ── QR code auth types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct QrCodeResp {
    pub qrcode: String,
    pub qrcode_img_content: String,
}

#[derive(Debug, Deserialize)]
pub struct QrStatusResp {
    pub status: String,
    pub bot_token: Option<String>,
    pub ilink_bot_id: Option<String>,
    pub baseurl: Option<String>,
}

// ── API functions ───────────────────────────────────────────────────

/// Long-poll for new messages. Returns empty response on timeout (normal).
pub async fn get_updates(
    client: &Client,
    base_url: &str,
    token: &str,
    buf: &str,
) -> Result<GetUpdatesResp> {
    let url = format!("{}/ilink/bot/getupdates", base_url.trim_end_matches('/'));
    let body = GetUpdatesReq {
        get_updates_buf: buf.to_string(),
        base_info: base_info(),
    };

    let resp = client
        .post(&url)
        .headers(build_headers(token))
        .json(&body)
        .timeout(LONG_POLL_TIMEOUT)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status();
            if !status.is_success() {
                bail!("getupdates HTTP {status}");
            }
            r.json().await.context("getupdates: invalid JSON")
        }
        Err(e) if e.is_timeout() => {
            // Long-poll timeout is normal — return empty response.
            Ok(GetUpdatesResp {
                ret: 0,
                errcode: None,
                errmsg: None,
                msgs: vec![],
                get_updates_buf: Some(buf.to_string()),
            })
        }
        Err(e) => Err(e.into()),
    }
}

/// Send a text message to a WeChat user.
pub async fn send_message(
    client: &Client,
    base_url: &str,
    token: &str,
    to_user_id: &str,
    context_token: &str,
    text: &str,
) -> Result<()> {
    let url = format!("{}/ilink/bot/sendmessage", base_url.trim_end_matches('/'));
    let body = SendMessageReqBody {
        msg: SendMessageMsg {
            to_user_id: to_user_id.to_string(),
            context_token: context_token.to_string(),
            message_type: 2,  // BOT
            message_state: 2, // FINISH
            item_list: vec![SendMessageItem {
                type_: 1, // TEXT
                text_item: SendTextItem {
                    text: text.to_string(),
                },
            }],
        },
        base_info: base_info(),
    };

    let r = client
        .post(&url)
        .headers(build_headers(token))
        .json(&body)
        .timeout(API_TIMEOUT)
        .send()
        .await?;

    if !r.status().is_success() {
        bail!("sendmessage HTTP {}", r.status());
    }
    Ok(())
}

/// Fetch a QR code for login.
pub async fn fetch_qrcode(client: &Client, base_url: &str) -> Result<QrCodeResp> {
    let url = format!(
        "{}/ilink/bot/get_bot_qrcode?bot_type=3",
        base_url.trim_end_matches('/')
    );
    let r = client.get(&url).timeout(API_TIMEOUT).send().await?;
    if !r.status().is_success() {
        bail!("get_bot_qrcode HTTP {}", r.status());
    }
    r.json().await.context("get_bot_qrcode: invalid JSON")
}

/// Long-poll QR code status until scanned/confirmed/expired.
pub async fn poll_qr_status(client: &Client, base_url: &str, qrcode: &str) -> Result<QrStatusResp> {
    let url = format!(
        "{}/ilink/bot/get_qrcode_status?qrcode={}",
        base_url.trim_end_matches('/'),
        qrcode
    );
    let resp = client
        .get(&url)
        .header("iLink-App-ClientVersion", "1")
        .timeout(LONG_POLL_TIMEOUT)
        .send()
        .await;

    match resp {
        Ok(r) => {
            if !r.status().is_success() {
                bail!("get_qrcode_status HTTP {}", r.status());
            }
            r.json().await.context("get_qrcode_status: invalid JSON")
        }
        Err(e) if e.is_timeout() => Ok(QrStatusResp {
            status: "wait".to_string(),
            bot_token: None,
            ilink_bot_id: None,
            baseurl: None,
        }),
        Err(e) => Err(e.into()),
    }
}

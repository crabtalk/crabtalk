//! HTTP transport (reqwest backend) — POST JSON-RPC with SSE response support.

use crate::client::sse;
use anyhow::{Context, Result, bail};

pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    session_id: Option<String>,
}

impl HttpTransport {
    pub fn new(url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.to_string(),
            session_id: None,
        }
    }

    pub async fn request(&mut self, msg: serde_json::Value) -> Result<serde_json::Value> {
        let mut req = self
            .client
            .post(self.url.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(sid) = self.session_id.as_deref() {
            req = req.header("Mcp-Session-Id", sid);
        }
        let resp = req.json(&msg).send().await.context("HTTP request failed")?;

        let status = resp.status();
        if let Some(sid) = resp.headers().get("mcp-session-id") {
            self.session_id = sid.to_str().ok().map(String::from);
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = resp.text().await.context("failed to read response body")?;

        if !status.is_success() {
            bail!("HTTP {status}: {body}");
        }

        if content_type.contains("text/event-stream") {
            sse::parse(&body)
        } else {
            serde_json::from_str(&body).context("invalid JSON response")
        }
    }

    pub async fn notify(&mut self, msg: serde_json::Value) -> Result<()> {
        let mut req = self
            .client
            .post(self.url.as_str())
            .header("Content-Type", "application/json");
        if let Some(sid) = self.session_id.as_deref() {
            req = req.header("Mcp-Session-Id", sid);
        }
        req.json(&msg).send().await.context("HTTP notify failed")?;
        Ok(())
    }
}

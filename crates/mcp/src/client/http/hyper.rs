//! HTTP transport (hyper backend) — POST JSON-RPC with SSE response support.

use crate::client::sse;
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;

/// Cap on MCP response bodies — protects against runaway servers. MCP
/// tool results are short; 16 MiB is generous.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

#[cfg(feature = "native-tls")]
type Connector = hyper_tls::HttpsConnector<HttpConnector>;
#[cfg(feature = "rustls")]
type Connector = hyper_rustls::HttpsConnector<HttpConnector>;

type HttpClient = Client<Connector, Full<Bytes>>;

pub struct HttpTransport {
    client: HttpClient,
    url: String,
    auth: Option<String>,
    session_id: Option<String>,
}

impl HttpTransport {
    pub fn new(url: &str, auth: Option<String>) -> Self {
        Self {
            client: Client::builder(TokioExecutor::new()).build(build_connector()),
            url: url.to_string(),
            auth,
            session_id: None,
        }
    }

    pub async fn request(&mut self, msg: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::to_vec(&msg)?;
        let mut builder = Request::post(self.url.as_str())
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(auth) = self.auth.as_deref() {
            builder = builder.header("authorization", auth);
        }
        if let Some(sid) = self.session_id.as_deref() {
            builder = builder.header("mcp-session-id", sid);
        }
        let req = builder.body(Full::new(Bytes::from(body)))?;

        let resp = self
            .client
            .request(req)
            .await
            .context("HTTP request failed")?;
        let status = resp.status();
        let session_id = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let bytes = Limited::new(resp.into_body(), MAX_BODY_BYTES)
            .collect()
            .await
            .map_err(|e| anyhow::anyhow!("failed to read response body: {e}"))?
            .to_bytes();
        let body = std::str::from_utf8(&bytes).context("response body not UTF-8")?;

        if !status.is_success() {
            bail!("HTTP {status}: {body}");
        }

        // Only persist session ID on a successful response.
        if let Some(sid) = session_id {
            self.session_id = Some(sid);
        }

        if content_type.contains("text/event-stream") {
            sse::parse(body)
        } else {
            serde_json::from_str(body).context("invalid JSON response")
        }
    }

    pub async fn notify(&mut self, msg: serde_json::Value) -> Result<()> {
        let body = serde_json::to_vec(&msg)?;
        let mut builder =
            Request::post(self.url.as_str()).header("content-type", "application/json");
        if let Some(auth) = self.auth.as_deref() {
            builder = builder.header("authorization", auth);
        }
        if let Some(sid) = self.session_id.as_deref() {
            builder = builder.header("mcp-session-id", sid);
        }
        let req = builder.body(Full::new(Bytes::from(body)))?;
        self.client
            .request(req)
            .await
            .context("HTTP notify failed")?;
        Ok(())
    }
}

#[cfg(feature = "native-tls")]
fn build_connector() -> Connector {
    hyper_tls::HttpsConnector::new()
}

#[cfg(feature = "rustls")]
fn build_connector() -> Connector {
    hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build()
}

//! SSE response parsing shared by both HTTP backends.

use anyhow::{Context, Result};

/// Extract the last JSON-RPC message from an SSE response body.
/// Takes only the final `data:` line — intermediate messages (progress
/// notifications, etc.) are intentionally skipped since we only use
/// request/response methods, not streaming notifications.
pub fn parse(body: &str) -> Result<serde_json::Value> {
    let mut last_data = None;
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if !data.is_empty() {
                last_data = Some(data);
            }
        }
    }
    let data = last_data.context("no data in SSE response")?;
    serde_json::from_str(data).context("invalid JSON in SSE data")
}

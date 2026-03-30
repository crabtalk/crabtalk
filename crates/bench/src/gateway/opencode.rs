//! OpenCode gateway — sends tasks via HTTP server API.

use crate::{
    gateway::{Gateway, TaskResult, timed},
    task::Task,
};
use reqwest::Client;
use serde_json::json;

pub struct OpenCodeGateway {
    base_url: String,
    client: Client,
}

impl OpenCodeGateway {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            client: Client::new(),
        }
    }
}

impl Gateway for OpenCodeGateway {
    fn run_task(&self, rt: &tokio::runtime::Runtime, task: &Task) -> TaskResult {
        let base_url = self.base_url.clone();
        let client = self.client.clone();
        let prompt = task.prompt.to_string();
        rt.block_on(async move {
            timed(|| async {
                // Create a new session.
                let session_resp: serde_json::Value = client
                    .post(format!("{base_url}/sessions"))
                    .json(&json!({}))
                    .send()
                    .await
                    .map_err(|e| format!("opencode create session failed: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("opencode session parse failed: {e}"))?;

                let session_id = session_resp["id"].as_str().unwrap_or("default").to_string();

                // Send message to session.
                let resp: serde_json::Value = client
                    .post(format!("{base_url}/sessions/{session_id}/messages"))
                    .json(&json!({ "content": prompt }))
                    .send()
                    .await
                    .map_err(|e| format!("opencode message failed: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("opencode response parse failed: {e}"))?;

                Ok(resp["content"]
                    .as_str()
                    .or(resp["response"].as_str())
                    .unwrap_or("")
                    .to_string())
            })
            .await
        })
    }
}

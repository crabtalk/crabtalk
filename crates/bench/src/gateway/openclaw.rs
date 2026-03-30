//! OpenClaw gateway — sends tasks via REST API (port 18789).

use crate::{
    gateway::{Gateway, TaskResult, timed},
    task::Task,
};
use reqwest::Client;
use serde_json::json;

pub struct OpenClawGateway {
    url: String,
    token: String,
    client: Client,
}

impl OpenClawGateway {
    pub fn new(port: u16, token: impl Into<String>) -> Self {
        Self {
            url: format!("http://127.0.0.1:{port}/api/sessions/main/messages"),
            token: token.into(),
            client: Client::new(),
        }
    }
}

impl Gateway for OpenClawGateway {
    fn run_task(&self, rt: &tokio::runtime::Runtime, task: &Task) -> TaskResult {
        let url = self.url.clone();
        let token = self.token.clone();
        let client = self.client.clone();
        let prompt = task.prompt.to_string();
        rt.block_on(async move {
            timed(|| async {
                let resp = client
                    .post(&url)
                    .bearer_auth(&token)
                    .json(&json!({ "content": prompt }))
                    .send()
                    .await
                    .map_err(|e| format!("openclaw request failed: {e}"))?;

                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| format!("openclaw response parse failed: {e}"))?;

                Ok(body["content"]
                    .as_str()
                    .or(body["response"].as_str())
                    .unwrap_or("")
                    .to_string())
            })
            .await
        })
    }
}

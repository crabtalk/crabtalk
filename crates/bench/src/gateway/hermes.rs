//! Hermes Agent gateway — sends tasks via OpenAI-compatible API.

use crate::{
    gateway::{Gateway, TaskResult, timed},
    task::Task,
};
use reqwest::Client;
use serde_json::json;

pub struct HermesGateway {
    url: String,
    client: Client,
}

impl HermesGateway {
    pub fn new(port: u16) -> Self {
        Self {
            url: format!("http://127.0.0.1:{port}/v1/chat/completions"),
            client: Client::new(),
        }
    }
}

impl Gateway for HermesGateway {
    fn run_task(&self, rt: &tokio::runtime::Runtime, task: &Task) -> TaskResult {
        let url = self.url.clone();
        let client = self.client.clone();
        let prompt = task.prompt.to_string();
        rt.block_on(async move {
            timed(|| async {
                let resp: serde_json::Value = client
                    .post(&url)
                    .json(&json!({
                        "model": "default",
                        "messages": [{ "role": "user", "content": prompt }]
                    }))
                    .send()
                    .await
                    .map_err(|e| format!("hermes request failed: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("hermes response parse failed: {e}"))?;

                Ok(resp["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string())
            })
            .await
        })
    }
}

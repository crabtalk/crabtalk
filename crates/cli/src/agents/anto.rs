//! Anto agent - a basic agent to verify tool calling

use anyhow::Result;
use chrono::Utc;
use cydonia::{Agent, Message, StreamChunk, Tool, ToolCall};
use schemars::JsonSchema;
use serde::Deserialize;

/// Anto - a basic agent with tools for testing tool calls
#[derive(Clone)]
pub struct Anto;

/// Parameters for the get_time tool
#[allow(dead_code)]
#[derive(JsonSchema, Deserialize)]
struct GetTimeParams {
    /// If returns UNIX timestamp instead
    timestamp: bool,
}

impl Agent for Anto {
    type Chunk = StreamChunk;

    fn system_prompt(&self) -> String {
        "You are Anto, a helpful assistant. You can get the current time.".into()
    }

    fn tools(&self) -> Vec<Tool> {
        vec![Tool {
            name: "get_time".into(),
            description: "Gets the current UTC time in ISO 8601 format.".into(),
            parameters: schemars::schema_for!(GetTimeParams),
            strict: true,
        }]
    }

    async fn dispatch(&self, tools: &[ToolCall]) -> Vec<Message> {
        tools
            .iter()
            .map(|call| {
                let result = match call.function.name.as_str() {
                    "get_time" => {
                        tracing::debug!("get_time arguments: {}", call.function.arguments);
                        let args = serde_json::from_str::<GetTimeParams>(&call.function.arguments)
                            .unwrap();
                        if args.timestamp {
                            Utc::now().timestamp().to_string()
                        } else {
                            Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
                        }
                    }
                    _ => format!("Unknown tool: {}", call.function.name),
                };
                Message::tool(result, call.id.clone())
            })
            .collect()
    }

    async fn chunk(&self, chunk: &StreamChunk) -> Result<Self::Chunk> {
        Ok(chunk.clone())
    }
}

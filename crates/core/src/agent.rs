//! Turbofish Agent library

use crate::{Message, StreamChunk, Tool, ToolCall};
use anyhow::Result;

/// A trait for turbofish agents
pub trait Agent: Clone {
    /// The parsed chunk from [StreamChunk]
    type Chunk;

    /// Agent identifier used for tool registration in teams.
    fn name(&self) -> &str {
        ""
    }

    /// Human-readable description shown to LLMs as tool description.
    fn description(&self) -> &str {
        ""
    }

    /// Build the system prompt for this agent.
    ///
    /// Called before each LLM request. Never stored in memory.
    fn system_prompt(&self) -> String;

    /// The tools for the agent
    fn tools(&self) -> Vec<Tool> {
        Vec::new()
    }

    /// Compact the chat history to reduce token usage.
    ///
    /// This method is called before each LLM request (both `send` and `stream`).
    /// Agents can override this to remove redundant data from historical messages,
    /// such as outdated candle data or large context that's no longer relevant.
    ///
    /// The default implementation returns messages unchanged.
    fn compact(&self, messages: Vec<Message>) -> Vec<Message> {
        messages
    }

    /// Dispatch tool calls
    fn dispatch(&self, tools: &[ToolCall]) -> impl Future<Output = Vec<Message>> {
        async move {
            tools
                .iter()
                .map(|tool| {
                    Message::tool(
                        format!("function {} not available", tool.function.name),
                        tool.id.clone(),
                    )
                })
                .collect()
        }
    }

    /// Parse a chunk from [StreamChunk]
    fn chunk(&self, chunk: &StreamChunk) -> impl Future<Output = Result<Self::Chunk>>;
}

impl Agent for () {
    type Chunk = StreamChunk;

    fn system_prompt(&self) -> String {
        "You are a helpful assistant.".into()
    }

    async fn chunk(&self, chunk: &StreamChunk) -> Result<Self::Chunk> {
        Ok(chunk.clone())
    }
}

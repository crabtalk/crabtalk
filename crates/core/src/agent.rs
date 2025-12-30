//! Turbofish Agent library

use crate::{Message, StreamChunk, Tool, ToolCall, ToolChoice};
use anyhow::Result;

/// A trait for turbofish agents
///
/// TODO: add schemar for request and response
pub trait Agent: Clone {
    /// The parsed chunk from [StreamChunk]
    type Chunk;

    /// The name of the agent
    const NAME: &str;

    /// The system prompt for the agent
    const SYSTEM_PROMPT: &str;

    /// The tools for the agent
    fn tools() -> Vec<Tool> {
        Vec::new()
    }

    /// Filter the messages to match required tools for the agent
    fn filter(&self, _message: &str) -> ToolChoice {
        ToolChoice::Auto
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

    const NAME: &str = "Default";

    const SYSTEM_PROMPT: &str = "You are a helpful assistant.";

    async fn chunk(&self, chunk: &StreamChunk) -> Result<Self::Chunk> {
        Ok(chunk.clone())
    }
}

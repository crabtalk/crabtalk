//! Chat abstractions for the unified LLM Interfaces

use crate::{
    Agent, Config, FinishReason, General, LLM, Response, Role, StreamChunk, ToolChoice,
    message::Message,
};
use anyhow::Result;
use futures_core::Stream;
use futures_util::StreamExt;

const MAX_TOOL_CALLS: usize = 16;

/// A chat for the LLM
#[derive(Clone)]
pub struct Chat<P: LLM, A: Agent> {
    /// The chat configuration
    pub config: P::ChatConfig,

    /// Conversation messages
    pub messages: Vec<Message>,

    /// The agent
    pub agent: A,

    /// The LLM provider
    provider: P,

    /// Whether to return the usage information in stream mode
    usage: bool,
}

impl<P: LLM, A: Agent> Chat<P, A> {
    /// Create a new chat
    pub fn new(config: General, provider: P, agent: A, messages: Vec<Message>) -> Self {
        let usage = config.usage;
        let config_typed: P::ChatConfig = config.into();
        let config_typed = config_typed.with_tools(agent.tools());
        Self {
            messages,
            provider,
            usage,
            agent,
            config: config_typed,
        }
    }

    /// Build the message list for an API request.
    ///
    /// Prepends the system prompt, applies agent-specific compaction,
    /// and strips reasoning content from non-tool-call messages (LLMs
    /// reject reasoning content on follow-up messages).
    fn api_messages(&self) -> Vec<Message> {
        let mut messages = self.messages.clone();

        // Prepend system prompt (built at runtime, never stored)
        if messages.first().map(|m| m.role) != Some(Role::System) {
            messages.insert(0, Message::system(self.agent.system_prompt()));
        }

        self.agent
            .compact(messages)
            .into_iter()
            .map(|mut m| {
                if m.tool_calls.is_empty() {
                    m.reasoning_content = String::new();
                }
                m
            })
            .collect()
    }

    /// Send a message to the LLM
    pub async fn send(&mut self, message: Message) -> Result<Response> {
        let mut config = self.config.clone().with_tool_choice(ToolChoice::Auto);
        self.messages.push(message);

        for _ in 0..MAX_TOOL_CALLS {
            let messages = self.api_messages();
            let response = self.provider.send(&config, &messages).await?;
            let Some(message) = response.message() else {
                return Ok(response);
            };

            if message.tool_calls.is_empty() {
                self.messages.push(message);
                return Ok(response);
            }

            let result = self.agent.dispatch(&message.tool_calls).await;
            self.messages.push(message);
            self.messages.extend(result);
            config = config.with_tool_choice(ToolChoice::None);
        }

        anyhow::bail!("max tool calls reached");
    }

    /// Send a message to the LLM with streaming
    pub fn stream(
        &mut self,
        message: Message,
    ) -> impl Stream<Item = Result<A::Chunk>> + use<'_, P, A> {
        let config = self.config.clone().with_tool_choice(ToolChoice::Auto);

        async_stream::try_stream! {
            self.messages.push(message);
            let mut config = config;

            for _ in 0..MAX_TOOL_CALLS {
                let messages = self.api_messages();
                let mut builder = Message::builder(Role::Assistant);

                // Stream the chunks
                let inner = self.provider.stream(config.clone(), &messages, self.usage);
                futures_util::pin_mut!(inner);
                while let Some(result) = inner.next().await {
                    let chunk = match result {
                        Ok(chunk) => chunk,
                        Err(e) => {
                            tracing::error!("Error in LLM stream: {:?}", e);
                            Err(e)?
                        }
                    };

                    if builder.accept(&chunk) {
                        yield self.agent.chunk(&chunk).await?;
                    }

                    if let Some(reason) = chunk.reason() {
                        match reason {
                            FinishReason::Stop => return,
                            FinishReason::ToolCalls => break,
                            reason => Err(anyhow::anyhow!("unexpected finish reason: {reason:?}"))?,
                        }
                    }
                }

                // Build the message and dispatch tool calls
                let message = builder.build();
                if message.tool_calls.is_empty() {
                    self.messages.push(message);
                    break;
                }

                yield self.agent.chunk(&StreamChunk::tool(&message.tool_calls)).await?;
                let result = self.agent.dispatch(&message.tool_calls).await;
                self.messages.push(message);
                self.messages.extend(result);
                config = config.with_tool_choice(ToolChoice::None);
            }
        }
    }
}

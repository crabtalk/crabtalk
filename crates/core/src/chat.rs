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

    /// Chat history in memory
    pub messages: Vec<Message>,

    /// The LLM provider
    provider: P,

    /// The agent
    agent: A,

    /// Whether to return the usage information in stream mode
    usage: bool,
}

impl<P: LLM> Chat<P, ()> {
    /// Create a new chat
    pub fn new(config: General, provider: P) -> Self {
        Self {
            messages: vec![],
            provider,
            usage: config.usage,
            agent: (),
            config: config.into(),
        }
    }
}

impl<P: LLM, A: Agent> Chat<P, A> {
    /// Get a mutable reference to the agent
    pub fn agent_mut(&mut self) -> &mut A {
        &mut self.agent
    }

    /// Get the chat messages for API requests.
    ///
    /// This applies agent-specific compaction to reduce token usage,
    /// then strips reasoning content from non-tool-call messages.
    pub fn messages(&self) -> Vec<Message> {
        self.agent
            .compact(self.messages.clone())
            .into_iter()
            .map(|mut m| {
                if m.tool_calls.is_empty() {
                    m.reasoning_content = String::new();
                }
                m
            })
            .collect()
    }

    /// Add the system prompt to the chat
    pub fn system<B: Agent>(mut self, agent: B) -> Chat<P, B> {
        let mut messages = self.messages;
        if messages.is_empty() {
            messages.push(Message::system(B::SYSTEM_PROMPT));
        } else if messages.first().map(|m| m.role) == Some(Role::System) {
            messages.insert(0, Message::system(B::SYSTEM_PROMPT));
        } else {
            messages = vec![Message::system(B::SYSTEM_PROMPT)]
                .into_iter()
                .chain(messages)
                .collect();
        }

        self.config = self.config.with_tools(B::tools());
        Chat {
            messages,
            provider: self.provider,
            usage: self.usage,
            agent,
            config: self.config,
        }
    }

    /// Send a message to the LLM
    pub async fn send(&mut self, message: Message) -> Result<Response> {
        let mut config = self
            .config
            .with_tool_choice(self.agent.filter(message.content.as_str()));
        self.messages.push(message);
        for _ in 0..MAX_TOOL_CALLS {
            let response = self.provider.send(&config, &self.messages()).await?;
            let Some(message) = response.message() else {
                return Ok(response);
            };

            if message.tool_calls.is_empty() {
                self.messages.push(message);
                return Ok(response);
            }

            let result = self.agent.dispatch(&message.tool_calls).await;
            self.messages.extend([vec![message], result].concat());
            config = config.with_tool_choice(ToolChoice::None);
        }

        anyhow::bail!("max tool calls reached");
    }

    /// Send a message to the LLM with streaming
    pub fn stream(
        &mut self,
        message: Message,
    ) -> impl Stream<Item = Result<A::Chunk>> + use<'_, P, A> {
        let mut config = self
            .config
            .with_tool_choice(self.agent.filter(message.content.as_str()));
        self.messages.push(message);

        async_stream::try_stream! {
            for _ in 0..MAX_TOOL_CALLS {
                let messages = self.messages();
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

                    if !builder.accept(&chunk) {
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
                self.messages.extend([vec![message], result].concat());
                config = config.with_tool_choice(ToolChoice::None);
            }
        }
    }
}

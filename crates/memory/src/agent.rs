//! Agent decorator that injects memory into the system prompt.

use crate::Memory;
use anyhow::Result;
use ccore::{Agent, Layer, Message, StreamChunk, Tool, ToolCall};

/// An agent wrapper that injects structured memory into the system prompt.
///
/// Delegates all [`Agent`] methods to the inner agent, but overrides
/// [`system_prompt()`](Agent::system_prompt) to append compiled memory
/// blocks after the base prompt.
///
/// # Example
///
/// ```rust,ignore
/// let agent = WithMemory::new(PerpAgent::new(pool, &req), memory);
/// let chat = Chat::new(config, provider, agent, messages);
/// // chat.agent.memory.set("user", "likes BTC");
/// ```
#[derive(Clone)]
pub struct WithMemory<A: Agent, M: Memory> {
    /// The inner agent.
    pub agent: A,
    /// The structured memory store.
    pub memory: M,
}

impl<A: Agent, M: Memory> WithMemory<A, M> {
    /// Create a new memory-enhanced agent.
    pub fn new(agent: A, memory: M) -> Self {
        Self { agent, memory }
    }
}

impl<A: Agent, M: Memory> Agent for WithMemory<A, M> {
    type Chunk = A::Chunk;

    fn name(&self) -> &str {
        self.agent.name()
    }

    fn description(&self) -> &str {
        self.agent.description()
    }

    fn system_prompt(&self) -> String {
        let base = self.agent.system_prompt();
        let mem = self.memory.compile();
        if mem.is_empty() {
            base
        } else {
            format!("{base}\n\n{mem}")
        }
    }

    fn tools(&self) -> Vec<Tool> {
        self.agent.tools()
    }

    fn compact(&self, messages: Vec<Message>) -> Vec<Message> {
        self.agent.compact(messages)
    }

    async fn dispatch(&self, tools: &[ToolCall]) -> Vec<Message> {
        self.agent.dispatch(tools).await
    }

    async fn chunk(&self, chunk: &StreamChunk) -> Result<Self::Chunk> {
        self.agent.chunk(chunk).await
    }
}

/// Memory layer — wraps any Agent and injects memory into its system prompt.
///
/// # Example
///
/// ```rust,ignore
/// use cydonia_memory::MemoryLayer;
///
/// let agent = MemoryLayer(memory).layer(inner_agent);
/// ```
#[derive(Clone)]
pub struct MemoryLayer<M: Memory>(pub M);

impl<A: Agent, M: Memory> Layer<A> for MemoryLayer<M> {
    type Agent = WithMemory<A, M>;

    fn layer(self, agent: A) -> Self::Agent {
        WithMemory::new(agent, self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemory;

    #[test]
    fn empty_memory_returns_base_prompt() {
        let agent = WithMemory::new((), InMemory::new());
        assert_eq!(agent.system_prompt(), "You are a helpful assistant.");
    }

    #[test]
    fn memory_appended_to_prompt() {
        let mut memory = InMemory::new();
        memory.set("user", "Likes Rust.");
        let agent = WithMemory::new((), memory);
        let prompt = agent.system_prompt();
        assert!(prompt.starts_with("You are a helpful assistant."));
        assert!(prompt.contains("<memory>"));
        assert!(prompt.contains("<user>\nLikes Rust.\n</user>"));
    }
}

//! Provider abstractions for the unified LLM Interfaces

use crate::{Config, Message, Response, StreamChunk};
use anyhow::Result;
use futures_core::Stream;
use reqwest::Client;

/// A trait for LLM providers
pub trait LLM: Sized + Clone {
    /// The chat configuration.
    type ChatConfig: Config + Send;

    /// Create a new LLM provider
    fn new(client: Client, key: &str) -> Result<Self>
    where
        Self: Sized;

    /// Send a message to the LLM
    fn send(
        &self,
        config: &Self::ChatConfig,
        messages: &[Message],
    ) -> impl Future<Output = Result<Response>> + Send;

    /// Send a message to the LLM with streaming
    fn stream(
        &self,
        config: Self::ChatConfig,
        messages: &[Message],
        usage: bool,
    ) -> impl Stream<Item = Result<StreamChunk>> + Send;
}

//! No-op LLM provider for testing.
//!
//! Implements [`LLM`] but panics on `send` and `stream`. Intended for
//! unit tests that exercise tool dispatch, memory, and session logic
//! without making real LLM calls.

use crate::{Client, General, LLM, Message, Response, StreamChunk};
use anyhow::Result;
use futures_core::Stream;

/// A no-op LLM provider that panics on any actual LLM call.
///
/// # Panics
///
/// Both `send` and `stream` panic if called. Only use this provider
/// in tests that never invoke LLM methods.
#[derive(Clone, Copy)]
pub struct NoopProvider;

impl LLM for NoopProvider {
    type ChatConfig = General;

    fn new(_client: Client, _key: &str) -> Result<Self> {
        Ok(Self)
    }

    async fn send(&self, _config: &General, _messages: &[Message]) -> Result<Response> {
        panic!("NoopProvider::send called — not intended for real LLM calls");
    }

    fn stream(
        &self,
        _config: General,
        _messages: &[Message],
        _usage: bool,
    ) -> impl Stream<Item = Result<StreamChunk>> {
        async_stream::stream! {
            panic!("NoopProvider::stream called — not intended for real LLM calls");
            #[allow(unreachable_code)]
            {
                yield Ok(StreamChunk::separator());
            }
        }
    }
}

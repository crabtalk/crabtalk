//! Context compaction — summarize session history and replace it.

use crate::agent::cancelled_fut;
use crabllm_core::{
    Provider,
    anthropic::{self, ContentBlock, ToolResultContent},
};
use store::HistoryEntry;
use tokio_util::sync::CancellationToken;

impl<P: Provider + 'static> super::Agent<P> {
    /// Summarize the session history using the LLM.
    ///
    /// `prompt` is the caller-supplied summarization instruction, sent as
    /// the closing user turn — the same shape a normal turn gives its
    /// history plus its new message (see [`super::Agent::build_request`]),
    /// so this request's prefix can hit the cache the live conversation
    /// already warmed. Returns the summary text, or `None` if the model
    /// produces no content or `cancel` fires before the model responds.
    pub async fn compact(
        &self,
        history: &[HistoryEntry],
        prompt: &str,
        cancel: Option<CancellationToken>,
    ) -> Option<String> {
        let model_name = self.config.model.clone();
        let system = if self.config.description.is_empty() {
            None
        } else {
            Some(anthropic::System::Blocks(vec![
                anthropic::ContentBlock::Text {
                    text: self.config.description.clone(),
                    cache_control: Some(serde_json::json!({"type": "ephemeral"})),
                },
            ]))
        };

        let max_len = self.config.compact_tool_max_len;
        let mut messages = Vec::with_capacity(history.len() + 1);
        for entry in history {
            let mut msg = entry.to_wire_message();
            for block in msg.blocks_mut().into_iter().flatten() {
                if let ContentBlock::ToolResult {
                    content: ToolResultContent::Text(text),
                    ..
                } = block
                    && text.len() > max_len
                {
                    text.truncate(text.floor_char_boundary(max_len));
                    text.push_str("... [truncated]");
                }
            }
            messages.push(msg);
        }
        messages.push(anthropic::Message {
            role: "user".to_string(),
            content: anthropic::Content::Text(prompt.to_owned()),
        });

        let request = anthropic::Request {
            model: model_name,
            messages,
            max_tokens: anthropic::DEFAULT_MAX_TOKENS,
            system,
            temperature: None,
            top_p: None,
            stream: None,
            tools: None,
            tool_choice: None,
            stop_sequences: None,
            thinking: None,
        };
        let response = tokio::select! {
            biased;
            _ = cancelled_fut(&cancel) => return None,
            result = self.model.send(request) => result,
        };
        match response {
            Ok(response) => response.content.iter().find_map(|b| match b {
                ContentBlock::Text { text, .. } if !text.is_empty() => Some(text.to_owned()),
                _ => None,
            }),
            Err(e) => {
                tracing::warn!("compaction LLM call failed: {e}");
                None
            }
        }
    }
}

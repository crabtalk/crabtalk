//! Context compaction — summarize conversation history and replace it.

use crate::model::HistoryEntry;
use crabllm_core::{
    AnthropicContent, AnthropicMessage, AnthropicRequest, AnthropicSystem, ContentBlock,
    DEFAULT_MAX_TOKENS, Provider, ToolResultContent,
};

pub(crate) const COMPACT_PROMPT: &str = include_str!("../../prompts/compact.md");

impl<P: Provider + 'static> super::Agent<P> {
    /// Summarize the conversation history using the LLM.
    ///
    /// Builds the base compact prompt, lets the `compact_hook` (if any) enrich
    /// it, then sends the history with the enriched prompt as system message.
    /// Returns the summary text, or `None` if the model produces no content.
    pub async fn compact(&self, history: &[HistoryEntry]) -> Option<String> {
        let model_name = self.config.model.clone();
        let prompt = COMPACT_PROMPT.to_owned();

        let mut messages = Vec::with_capacity(1 + history.len());
        if !self.config.system_prompt.is_empty() {
            messages.push(AnthropicMessage {
                role: "user".to_string(),
                content: AnthropicContent::Text(format!(
                    "Agent system prompt (preserve identity/profile info):\n{}",
                    self.config.system_prompt
                )),
            });
        }
        let max_len = self.config.compact_tool_max_len;
        for entry in history {
            let mut msg = entry.to_wire_message();
            for block in &mut msg.content {
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
            messages.push(AnthropicMessage {
                role: msg.role.as_str().to_string(),
                content: AnthropicContent::Blocks(msg.content),
            });
        }

        let request = AnthropicRequest {
            model: model_name,
            messages,
            max_tokens: DEFAULT_MAX_TOKENS,
            system: Some(AnthropicSystem::Text(prompt)),
            temperature: None,
            top_p: None,
            stream: None,
            tools: None,
            tool_choice: None,
            stop_sequences: None,
            thinking: None,
        };
        match self.model.send(request).await {
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

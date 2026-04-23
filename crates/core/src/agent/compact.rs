//! Context compaction — summarize conversation history and replace it.

use crate::model::HistoryEntry;
use crabllm_core::{ChatCompletionRequest, Message, Provider, Role};

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

        let mut messages = Vec::with_capacity(2 + history.len());
        messages.push(Message::system(&prompt));
        // Include the agent's system prompt as identity context so the
        // compaction LLM preserves <self>, <identity>, and <profile> info.
        if !self.config.system_prompt.is_empty() {
            messages.push(Message::user(format!(
                "Agent system prompt (preserve identity/profile info):\n{}",
                self.config.system_prompt
            )));
        }
        let max_len = self.config.compact_tool_max_len;
        for entry in history {
            let mut msg = entry.to_wire_message();
            if *entry.role() == Role::Tool
                && let Some(serde_json::Value::String(text)) = msg.content.as_mut()
                && text.len() > max_len
            {
                text.truncate(text.floor_char_boundary(max_len));
                text.push_str("... [truncated]");
            }
            messages.push(msg);
        }

        let request = ChatCompletionRequest {
            model: model_name,
            messages,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: None,
            stop: None,
            tools: None,
            tool_choice: None,
            frequency_penalty: None,
            presence_penalty: None,
            seed: None,
            user: None,
            reasoning_effort: None,
            thinking: None,
            anthropic_max_tokens: None,
            extra: Default::default(),
        };
        match self.model.send_ct(request).await {
            Ok(response) => response.content().map(|s| s.to_owned()),
            Err(e) => {
                tracing::warn!("compaction LLM call failed: {e}");
                None
            }
        }
    }

    /// Estimate the token count of conversation history.
    ///
    /// Uses a simple heuristic: ~4 characters per token. Counts content,
    /// reasoning_content, and tool call arguments.
    pub(crate) fn estimate_tokens(history: &[HistoryEntry]) -> usize {
        history.iter().map(|e| e.estimate_tokens()).sum()
    }
}
